//! Environment on Windows: PATH/JAVA_HOME in the registry, change broadcasting and
//! elevated relaunch.
//!
//! We edit the registry directly — we do NOT use `setx`, which truncates the PATH to
//! 1024 characters. When rewriting `Path` we PRESERVE its type (`REG_EXPAND_SZ`):
//! downgrading it to `REG_SZ` would stop expanding references like `%SystemRoot%`
//! already present and would break the user's PATH.
//!
//! Reminder about effective PATH priority: SYSTEM first, USER
//! after. That is why prepending to the user PATH does NOT beat a JDK that is
//! in the system PATH (typical case: Oracle's `javapath`); for that there is
//! `setup --system`, which edits `HKLM` (requires elevation).

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

/// User environment subkey (`HKCU\Environment`).
const USER_ENV: &str = "Environment";
/// System environment subkey (`HKLM\...\Session Manager\Environment`).
const SYSTEM_ENV: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

/// Scope where the environment changes are applied.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// `HKCU\Environment` — no UAC, covers most cases.
    User,
    /// `HKLM\...\Environment` — requires elevation, beats the user PATH.
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
            .with_context(|| format!("could not open the environment key '{sub}'"))
    }
}

/// Encodes a string to UTF-16LE with a null terminator (format of a
/// `REG_SZ`/`REG_EXPAND_SZ` value).
fn encode_wide_bytes(s: &str) -> Vec<u8> {
    s.encode_utf16()
        .chain(once(0))
        .flat_map(|u| u.to_le_bytes())
        .collect()
}

/// Reads a string value from the registry as `(text, type)`, or `None` if it does not exist.
fn read_value(key: &RegKey, name: &str) -> Result<Option<(String, RegType)>> {
    match key.get_raw_value(name) {
        Ok(raw) => {
            let text = String::from_reg_value(&raw)
                .with_context(|| format!("the registry value '{name}' is not text"))?;
            Ok(Some((text, raw.vtype)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading '{name}' from the registry")),
    }
}

/// Writes a string value preserving the given `vtype`.
fn write_value(key: &RegKey, name: &str, value: &str, vtype: RegType) -> Result<()> {
    let raw = RegValue {
        bytes: Cow::Owned(encode_wide_bytes(value)),
        vtype,
    };
    key.set_raw_value(name, &raw)
        .with_context(|| format!("writing '{name}' to the registry"))
}

/// Normalizes a PATH entry for comparison ignoring case and trailing
/// backslash (Windows path semantics).
fn normalize(p: &str) -> String {
    p.trim().trim_end_matches('\\').to_lowercase()
}

/// Prepends `entries` (in order) to the `scope` PATH, without duplicating and without touching
/// the rest. Returns `true` if there were changes. Idempotent: if the entries are
/// already at the front, it does not rewrite anything.
pub fn prepend_path(scope: Scope, entries: &[&str]) -> Result<bool> {
    let key = scope.open(KEY_READ | KEY_SET_VALUE)?;
    let (current, vtype) = match read_value(&key, "Path")? {
        Some(v) => v,
        // Nonexistent PATH (new account): we create it as REG_EXPAND_SZ, the
        // canonical type for PATH on Windows.
        None => (String::new(), REG_EXPAND_SZ),
    };

    let prepend_norm: Vec<String> = entries.iter().map(|e| normalize(e)).collect();
    let mut new_parts: Vec<String> = entries.iter().map(|e| (*e).to_string()).collect();
    // Keep the rest of the PATH, removing the entries we just prepended
    // (avoids duplicates and "promotes" them to the front if they were already further back).
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

/// Sets (or corrects) `JAVA_HOME` in the `scope`. Returns `true` if it changed.
pub fn set_java_home(scope: Scope, value: &str) -> Result<bool> {
    let key = scope.open(KEY_READ | KEY_SET_VALUE)?;
    if let Some((current, _)) = read_value(&key, "JAVA_HOME")?
        && normalize(&current) == normalize(value)
    {
        return Ok(false);
    }
    // JAVA_HOME is an absolute path without variables → REG_SZ.
    write_value(&key, "JAVA_HOME", value, REG_SZ)?;
    Ok(true)
}

/// Result of attempting to undo `JAVA_HOME` (used by `setup --undo`).
pub enum JavaHomeUndo {
    /// Pointed to jdkenv and was removed.
    Removed,
    /// Was not defined.
    NotSet,
    /// Points to something else (not jdkenv's): we leave it intact.
    KeptForeign(String),
}

/// Removes the given `entries` from the `scope` PATH (normalized comparison),
/// leaving the rest intact and preserving the vtype. Returns `true` if it removed
/// something. It is the inverse operation of [`prepend_path`].
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
            // Discard only jdkenv's entries; everything else (including
            // empty entries) is kept verbatim to avoid altering more than needed.
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

/// If `JAVA_HOME` points to `expected` (jdkenv's `current`), it removes it. If
/// it points to something else, it leaves it intact: we do not overwrite a foreign `JAVA_HOME`
/// that the user might have had before.
pub fn clear_java_home_if(scope: Scope, expected: &str) -> Result<JavaHomeUndo> {
    let key = scope.open(KEY_READ | KEY_SET_VALUE)?;
    match read_value(&key, "JAVA_HOME")? {
        None => Ok(JavaHomeUndo::NotSet),
        Some((current, _)) if normalize(&current) == normalize(expected) => {
            key.delete_value("JAVA_HOME")
                .context("could not remove JAVA_HOME from the registry")?;
            Ok(JavaHomeUndo::Removed)
        }
        Some((current, _)) => Ok(JavaHomeUndo::KeptForeign(current)),
    }
}

/// Reads the `scope` PATH without expanding variables. `None` if it does not exist.
pub fn read_path(scope: Scope) -> Result<Option<String>> {
    Ok(read_value(&scope.open(KEY_READ)?, "Path")?.map(|(s, _)| s))
}

/// Reads `JAVA_HOME` from the `scope`. `None` if it does not exist.
pub fn read_java_home(scope: Scope) -> Result<Option<String>> {
    Ok(read_value(&scope.open(KEY_READ)?, "JAVA_HOME")?.map(|(s, _)| s))
}

/// Notifies top-level windows that the environment changed, so that new
/// shells pick up the PATH/JAVA_HOME without logging out. `lParam = "Environment"`
/// tells the receiver which section to re-read.
pub fn broadcast_env_change() {
    let section: Vec<u16> = OsStr::new("Environment")
        .encode_wide()
        .chain(once(0))
        .collect();
    let mut result: usize = 0;
    // SMTO_ABORTIFHUNG + short timeout: we do not block if some window hangs.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0, // wParam unused for WM_SETTINGCHANGE
            section.as_ptr() as LPARAM,
            SMTO_ABORTIFHUNG,
            5000,
            &mut result,
        );
    }
}

/// Does the process run with administrator privileges (elevated token)?
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

/// Encodes an `OsStr` to a null-terminated wide string (for Win32 `*W`).
fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(once(0)).collect()
}

/// Quotes an argument if it contains spaces, to pass it through the command line.
fn quote_arg(a: &str) -> String {
    if a.is_empty() || a.contains([' ', '\t', '"']) {
        format!("\"{}\"", a.replace('"', "\\\""))
    } else {
        a.to_string()
    }
}

/// Relaunches this same executable with elevation (UAC) and the given `args`, waits
/// for it to finish and returns its exit code. Used by `setup --system` when
/// not running as administrator.
pub fn relaunch_elevated(args: &[String]) -> Result<i32> {
    let exe = std::env::current_exe().context("could not get the executable path")?;
    let exe_w = wide(exe.as_os_str());
    let verb_w = wide(OsStr::new("runas"));
    let params = args
        .iter()
        .map(|a| quote_arg(a))
        .collect::<Vec<_>>()
        .join(" ");
    let params_w = wide(OsStr::new(&params));

    // SAFETY: zeroed() leaves valid null pointers in the fields we do not use.
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS; // we want the process handle to wait on
    info.lpVerb = verb_w.as_ptr();
    info.lpFile = exe_w.as_ptr();
    info.lpParameters = params_w.as_ptr();
    info.nShow = SW_SHOWNORMAL;

    let ok = unsafe { ShellExecuteExW(&mut info) };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        // 1223 = ERROR_CANCELLED: the user rejected the UAC prompt.
        if err == 1223 {
            bail!("elevation was cancelled (UAC). `setup --system` requires administrator permissions.");
        }
        bail!("could not relaunch with elevation (error code {err}).");
    }

    let handle = info.hProcess;
    if handle.is_null() {
        // Without a handle (despite SEE_MASK_NOCLOSEPROCESS) we cannot confirm that the
        // elevated process finished correctly; we do not fake silent success.
        bail!("did not obtain the handle of the elevated process; could not confirm `setup --system`.");
    }
    let code = unsafe {
        // INFINITE should not expire, but we validate the return: a WAIT_FAILED
        // would leave the handle in an invalid state and GetExitCodeProcess would return garbage.
        let wait = WaitForSingleObject(handle, INFINITE);
        if wait != WAIT_OBJECT_0 {
            CloseHandle(handle);
            bail!("failed waiting for the elevated process (WaitForSingleObject returned {wait}).");
        }
        let mut code: u32 = 0;
        let got = GetExitCodeProcess(handle, &mut code);
        if got == 0 {
            let err = GetLastError();
            CloseHandle(handle);
            bail!("could not get the exit code of the elevated process (error {err}).");
        }
        CloseHandle(handle);
        code as i32
    };
    Ok(code)
}
