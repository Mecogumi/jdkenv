//! Entorno en Windows: PATH/JAVA_HOME en el registro, broadcast de cambios y
//! relanzado elevado.
//!
//! Editamos el registro directamente — NO usamos `setx`, que trunca el PATH a
//! 1024 caracteres. Al reescribir `Path` PRESERVAMOS su tipo (`REG_EXPAND_SZ`):
//! degradarlo a `REG_SZ` dejaría de expandir referencias como `%SystemRoot%` ya
//! presentes y rompería el PATH del usuario.
//!
//! Recordatorio de prioridad del PATH efectivo: SISTEMA primero, USUARIO
//! después. Por eso anteponer en el PATH de usuario NO vence a un JDK que esté
//! en el PATH de sistema (caso típico: el `javapath` de Oracle); para eso está
//! `setup --system`, que edita `HKLM` (requiere elevación).

use std::borrow::Cow;
use std::ffi::OsStr;
use std::iter::once;
use std::os::windows::ffi::OsStrExt;

use anyhow::{bail, Context, Result};
use winreg::enums::{
    RegType, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ,
};
use winreg::types::FromRegValue;
use winreg::{RegKey, RegValue};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, LPARAM, WAIT_OBJECT_0};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, WaitForSingleObject, INFINITE,
};
use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, SW_SHOWNORMAL, WM_SETTINGCHANGE,
};

/// Subclave de entorno del usuario (`HKCU\Environment`).
const USER_ENV: &str = "Environment";
/// Subclave de entorno del sistema (`HKLM\...\Session Manager\Environment`).
const SYSTEM_ENV: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

/// Ámbito donde se aplican los cambios de entorno.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// `HKCU\Environment` — sin UAC, cubre la mayoría de los casos.
    User,
    /// `HKLM\...\Environment` — requiere elevación, gana al PATH de usuario.
    System,
}

impl Scope {
    fn root_and_sub(self) -> (winreg::HKEY, &'static str) {
        match self {
            Scope::User => (HKEY_CURRENT_USER, USER_ENV),
            Scope::System => (HKEY_LOCAL_MACHINE, SYSTEM_ENV),
        }
    }

    fn open(self, perms: u32) -> Result<RegKey> {
        let (root, sub) = self.root_and_sub();
        RegKey::predef(root)
            .open_subkey_with_flags(sub, perms)
            .with_context(|| format!("no se pudo abrir la clave de entorno '{sub}'"))
    }
}

/// Codifica una cadena a UTF-16LE con terminador nulo (formato de un valor
/// `REG_SZ`/`REG_EXPAND_SZ`).
fn encode_wide_bytes(s: &str) -> Vec<u8> {
    s.encode_utf16()
        .chain(once(0))
        .flat_map(|u| u.to_le_bytes())
        .collect()
}

/// Lee un valor de cadena del registro como `(texto, tipo)`, o `None` si no existe.
fn read_value(key: &RegKey, name: &str) -> Result<Option<(String, RegType)>> {
    match key.get_raw_value(name) {
        Ok(raw) => {
            let text = String::from_reg_value(&raw)
                .with_context(|| format!("el valor de registro '{name}' no es texto"))?;
            Ok(Some((text, raw.vtype)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("leyendo '{name}' del registro")),
    }
}

/// Escribe un valor de cadena preservando el `vtype` indicado.
fn write_value(key: &RegKey, name: &str, value: &str, vtype: RegType) -> Result<()> {
    let raw = RegValue {
        bytes: Cow::Owned(encode_wide_bytes(value)),
        vtype,
    };
    key.set_raw_value(name, &raw)
        .with_context(|| format!("escribiendo '{name}' en el registro"))
}

/// Normaliza una ruta de PATH para comparar sin distinguir mayúsculas ni barra
/// final (semántica de rutas en Windows).
fn normalize(p: &str) -> String {
    p.trim().trim_end_matches('\\').to_lowercase()
}

/// Antepone `entries` (en orden) al PATH del `scope`, sin duplicar y sin tocar
/// el resto. Devuelve `true` si hubo cambios. Idempotente: si las entradas ya
/// están al frente, no reescribe nada.
pub fn prepend_path(scope: Scope, entries: &[&str]) -> Result<bool> {
    let key = scope.open(KEY_READ | KEY_SET_VALUE)?;
    let (current, vtype) = match read_value(&key, "Path")? {
        Some(v) => v,
        // PATH inexistente (cuenta nueva): lo creamos como REG_EXPAND_SZ, el tipo
        // canónico para PATH en Windows.
        None => (String::new(), REG_EXPAND_SZ),
    };

    let prepend_norm: Vec<String> = entries.iter().map(|e| normalize(e)).collect();
    let mut new_parts: Vec<String> = entries.iter().map(|e| (*e).to_string()).collect();
    // Conserva el resto del PATH, quitando las entradas que acabamos de anteponer
    // (evita duplicados y las "promociona" al frente si ya estaban más atrás).
    for part in current.split(';') {
        if part.is_empty() {
            continue;
        }
        if !prepend_norm.contains(&normalize(part)) {
            new_parts.push(part.to_string());
        }
    }
    let new_path = new_parts.join(";");

    if normalize(&new_path) == normalize(&current) {
        return Ok(false);
    }
    write_value(&key, "Path", &new_path, vtype)?;
    Ok(true)
}

/// Establece (o corrige) `JAVA_HOME` en el `scope`. Devuelve `true` si cambió.
pub fn set_java_home(scope: Scope, value: &str) -> Result<bool> {
    let key = scope.open(KEY_READ | KEY_SET_VALUE)?;
    if let Some((current, _)) = read_value(&key, "JAVA_HOME")?
        && normalize(&current) == normalize(value)
    {
        return Ok(false);
    }
    // JAVA_HOME es una ruta absoluta sin variables → REG_SZ.
    write_value(&key, "JAVA_HOME", value, REG_SZ)?;
    Ok(true)
}

/// Resultado de intentar deshacer `JAVA_HOME` (lo usa `setup --undo`).
pub enum JavaHomeUndo {
    /// Apuntaba a jdkenv y se eliminó.
    Removed,
    /// No estaba definido.
    NotSet,
    /// Apunta a otra cosa (no de jdkenv): lo dejamos intacto.
    KeptForeign(String),
}

/// Quita del PATH del `scope` las `entries` indicadas (comparación normalizada),
/// dejando el resto intacto y preservando el vtype. Devuelve `true` si quitó
/// algo. Es la operación inversa de [`prepend_path`].
pub fn remove_from_path(scope: Scope, entries: &[&str]) -> Result<bool> {
    let key = scope.open(KEY_READ | KEY_SET_VALUE)?;
    let Some((current, vtype)) = read_value(&key, "Path")? else {
        return Ok(false);
    };
    let remove_norm: Vec<String> = entries.iter().map(|e| normalize(e)).collect();

    let mut removed_any = false;
    let kept: Vec<&str> = current
        .split(';')
        .filter(|part| {
            // Descarta solo las entradas de jdkenv; todo lo demás (incluidas
            // entradas vacías) se conserva verbatim para no alterar de más.
            if !part.is_empty() && remove_norm.contains(&normalize(part)) {
                removed_any = true;
                false
            } else {
                true
            }
        })
        .collect();

    if !removed_any {
        return Ok(false);
    }
    write_value(&key, "Path", &kept.join(";"), vtype)?;
    Ok(true)
}

/// Si `JAVA_HOME` apunta a `expected` (el `current` de jdkenv), lo elimina. Si
/// apunta a otra cosa, lo deja intacto: no pisamos un `JAVA_HOME` ajeno que el
/// usuario pudiera tener de antes.
pub fn clear_java_home_if(scope: Scope, expected: &str) -> Result<JavaHomeUndo> {
    let key = scope.open(KEY_READ | KEY_SET_VALUE)?;
    match read_value(&key, "JAVA_HOME")? {
        None => Ok(JavaHomeUndo::NotSet),
        Some((current, _)) if normalize(&current) == normalize(expected) => {
            key.delete_value("JAVA_HOME")
                .context("no se pudo eliminar JAVA_HOME del registro")?;
            Ok(JavaHomeUndo::Removed)
        }
        Some((current, _)) => Ok(JavaHomeUndo::KeptForeign(current)),
    }
}

/// Lee el PATH del `scope` sin expandir variables. `None` si no existe.
pub fn read_path(scope: Scope) -> Result<Option<String>> {
    Ok(read_value(&scope.open(KEY_READ)?, "Path")?.map(|(s, _)| s))
}

/// Lee `JAVA_HOME` del `scope`. `None` si no existe.
pub fn read_java_home(scope: Scope) -> Result<Option<String>> {
    Ok(read_value(&scope.open(KEY_READ)?, "JAVA_HOME")?.map(|(s, _)| s))
}

/// Notifica a las ventanas de nivel superior que el entorno cambió, para que las
/// shells nuevas tomen el PATH/JAVA_HOME sin cerrar sesión. `lParam = "Environment"`
/// le indica al receptor qué sección releer.
pub fn broadcast_env_change() {
    let section: Vec<u16> = OsStr::new("Environment")
        .encode_wide()
        .chain(once(0))
        .collect();
    let mut result: usize = 0;
    // SMTO_ABORTIFHUNG + timeout corto: no nos bloqueamos si alguna ventana cuelga.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0, // wParam sin uso para WM_SETTINGCHANGE
            section.as_ptr() as LPARAM,
            SMTO_ABORTIFHUNG,
            5000,
            &mut result,
        );
    }
}

/// ¿El proceso corre con privilegios de administrador (token elevado)?
pub fn is_elevated() -> bool {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut ret_len: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

/// Codifica un `OsStr` a una cadena ancha terminada en nulo (para Win32 `*W`).
fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(once(0)).collect()
}

/// Cita un argumento si contiene espacios, para pasarlo por la línea de comandos.
fn quote_arg(a: &str) -> String {
    if a.is_empty() || a.contains([' ', '\t', '"']) {
        format!("\"{}\"", a.replace('"', "\\\""))
    } else {
        a.to_string()
    }
}

/// Relanza este mismo ejecutable con elevación (UAC) y los `args` dados, espera
/// a que termine y devuelve su código de salida. Lo usa `setup --system` cuando
/// no se está corriendo como administrador.
pub fn relaunch_elevated(args: &[String]) -> Result<i32> {
    let exe = std::env::current_exe().context("no se pudo obtener la ruta del ejecutable")?;
    let exe_w = wide(exe.as_os_str());
    let verb_w = wide(OsStr::new("runas"));
    let params = args
        .iter()
        .map(|a| quote_arg(a))
        .collect::<Vec<_>>()
        .join(" ");
    let params_w = wide(OsStr::new(&params));

    // SAFETY: zeroed() deja punteros nulos válidos en los campos que no usamos.
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS; // queremos el handle del proceso para esperar
    info.lpVerb = verb_w.as_ptr();
    info.lpFile = exe_w.as_ptr();
    info.lpParameters = params_w.as_ptr();
    info.nShow = SW_SHOWNORMAL;

    let ok = unsafe { ShellExecuteExW(&mut info) };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        // 1223 = ERROR_CANCELLED: el usuario rechazó el prompt de UAC.
        if err == 1223 {
            bail!("se canceló la elevación (UAC). `setup --system` necesita permisos de administrador.");
        }
        bail!("no se pudo relanzar con elevación (código de error {err}).");
    }

    let handle = info.hProcess;
    if handle.is_null() {
        // Sin handle (pese a SEE_MASK_NOCLOSEPROCESS) no podemos confirmar que el
        // proceso elevado terminó bien; no fingimos éxito silencioso.
        bail!("no se obtuvo el handle del proceso elevado; no se pudo confirmar `setup --system`.");
    }
    let code = unsafe {
        // INFINITE no debería expirar, pero validamos el retorno: un WAIT_FAILED
        // dejaría el handle en estado inválido y GetExitCodeProcess daría basura.
        let wait = WaitForSingleObject(handle, INFINITE);
        if wait != WAIT_OBJECT_0 {
            CloseHandle(handle);
            bail!("fallo esperando al proceso elevado (WaitForSingleObject devolvió {wait}).");
        }
        let mut code: u32 = 0;
        let got = GetExitCodeProcess(handle, &mut code);
        if got == 0 {
            let err = GetLastError();
            CloseHandle(handle);
            bail!("no se pudo obtener el código de salida del proceso elevado (error {err}).");
        }
        CloseHandle(handle);
        code as i32
    };
    Ok(code)
}
