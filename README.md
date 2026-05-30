# jdkenv

Gestor de versiones de **JDK para Windows**, nativo (PowerShell/cmd, sin WSL ni
Git Bash). Descarga JDKs desde la [foojay Disco API](https://api.foojay.io/) y
cambia la versión activa al instante, sin reiniciar la terminal.

Inspirado en `pyenv`/`jenv`, implementado en Rust como un único `.exe` sin
dependencias externas (TLS vía rustls, no requiere OpenSSL).

---

## Instalación de un comando

```powershell
irm https://<MI_DOMINIO>/install.ps1 | iex
```

Esto descarga el binario adecuado (x64/arm64) a `%USERPROFILE%\.jdkenv\bin\` y
ejecuta `jdkenv setup` para registrar `PATH` y `JAVA_HOME`. Abre **una terminal
nueva** y listo.

> **Nota de seguridad (honesta):** `irm | iex` ejecuta código remoto sin
> verificar, igual que `curl | bash`. Es el patrón estándar de Scoop/mise, pero
> implica confiar en la fuente (`<MI_DOMINIO>`). Si prefieres, descarga el
> `.exe` desde [Releases](https://github.com/<USER>/jdkenv/releases/latest),
> colócalo en `%USERPROFILE%\.jdkenv\bin\jdkenv.exe` y ejecuta `jdkenv setup`.

---

## Comandos

| Comando | Qué hace |
|---|---|
| `jdkenv install <version> [--distribution <dist>]` | Descarga e instala un JDK (`.zip`) desde foojay. `--distribution` por defecto `temurin`. El primer JDK instalado se activa solo. |
| `jdkenv global <version>` | Activa una versión instalada (re-apunta el junction `current`). |
| `jdkenv list` | Lista las versiones instaladas (`*` = activa). |
| `jdkenv list --remote [--distribution <dist>]` | Lista versiones disponibles en foojay para Windows + tu arquitectura. |
| `jdkenv uninstall <version>` | Borra una versión. Se niega si es la activa. |
| `jdkenv current` (alias `which`) | Muestra la versión activa y a qué carpeta apunta `current`. |
| `jdkenv setup [--system]` | Registra `PATH`/`JAVA_HOME`. Sin flag: PATH de usuario. `--system`: PATH de sistema (pide elevación). |
| `jdkenv doctor` | Diagnostica el entorno y detecta otro `java.exe` que te gane en el PATH. |
| `jdkenv local <version>` | *(v2, no implementado)* Versión por carpeta. |

### Ejemplos

```powershell
jdkenv install 21                          # Temurin 21 (última build)
jdkenv install 17 --distribution corretto  # Corretto 17
jdkenv list --remote --distribution zulu   # ¿qué Zulu hay disponible?
jdkenv global 17                            # cambia el JDK activo
jdkenv current                              # ¿cuál está activo?
jdkenv doctor                               # ¿algo me está ganando en el PATH?
```

Las versiones aceptan prefijos: `21` resuelve a la build más reciente de esa
línea (p.ej. `21.0.11+10`); también puedes ser específico (`17.0.13`).

---

## El modelo de junction (lo que lo hace instantáneo)

El árbol vive bajo `%USERPROFILE%\.jdkenv\`:

```
.jdkenv\
├── bin\
│   └── jdkenv.exe
├── versions\
│   ├── temurin-21.0.11+10\
│   └── corretto-17.0.13\
└── current\            ← junction NTFS → una carpeta dentro de versions\
```

La clave: `PATH` y `JAVA_HOME` apuntan **siempre** a `current`, nunca a una
versión concreta:

- `PATH` contiene la ruta literal `...\.jdkenv\current\bin`
- `JAVA_HOME = ...\.jdkenv\current`

Cambiar de versión (`jdkenv global <v>`) solo **re-apunta** el junction
`current` → `versions\<dist>-<version>`. Como `current\bin` es una ruta literal
del PATH y `current` es un junction:

- Las terminales **ya abiertas** toman la versión nueva en el siguiente `java`
  que lancen — sin reiniciar ni rehacer el broadcast.
- `JAVA_HOME` sigue siendo correcto sin tocar el registro.

Se usa un **directory junction** (no un symlink) a propósito: los junctions
**no requieren permisos de administrador** ni Developer Mode. Re-apuntar borra
el junction (esto **no** borra el target — un junction es solo un reparse point)
y lo crea de nuevo apuntando a la versión elegida.

---

## Los dos PATH: usuario vs sistema (importante)

En Windows hay dos PATH y el efectivo se compone **SISTEMA primero, USUARIO
después**:

- PATH de **usuario** → `HKCU\Environment`
- PATH de **sistema** → `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment`

Consecuencia: anteponer en el PATH de **usuario** (lo que hace `jdkenv setup`
por defecto) **no vence** a un `java.exe` que esté en el PATH de **sistema** — el
caso típico es el `javapath` de Oracle, que el instalador de Oracle deja en el
PATH de sistema.

- `jdkenv setup` (por defecto) edita solo `HKCU`: cubre la mayoría de casos sin
  pedir UAC.
- `jdkenv setup --system` edita `HKLM` y antepone ahí para **prioridad
  absoluta**. Requiere elevación; si no corres como admin, jdkenv se **relanza
  elevado** (UAC) con los mismos argumentos.
- `jdkenv doctor` te dice cuándo necesitas `--system` (detecta qué `java.exe`
  gana realmente en tu PATH).

> Muchas build tools (**Maven**, **Gradle**) priorizan `JAVA_HOME` sobre el
> PATH. Como `setup` también setea `JAVA_HOME → current`, gran parte de los
> flujos funcionan aunque el orden del PATH no sea perfecto.

Detalles de implementación en Windows:
- Editamos el registro **directamente**, no con `setx` (que trunca el PATH a
  1024 caracteres).
- Al reescribir `Path` **preservamos su tipo** (`REG_EXPAND_SZ`): degradarlo a
  `REG_SZ` rompería referencias como `%SystemRoot%` ya presentes.
- Tras escribir, hacemos *broadcast* de `WM_SETTINGCHANGE` para que las
  terminales nuevas tomen el cambio sin cerrar sesión.

---

## Compilar desde el código

Requiere [Rust](https://rustup.rs/) (toolchain MSVC).

```powershell
cargo build --release
# binario en target\release\jdkenv.exe

# Targets soportados:
#   x86_64-pc-windows-msvc   (x64)
#   aarch64-pc-windows-msvc  (ARM64)
```

El proyecto compila sin warnings y pasa `cargo clippy`.

### Estructura

```
src/
├── main.rs        # parser clap + dispatch
├── arch.rs        # detección x64/aarch64 → parámetro de foojay
├── paths.rs       # layout .jdkenv, junction (crear/re-apuntar), versiones instaladas
├── foojay.rs      # cliente Disco API + descarga/extracción del .zip
├── env_win.rs     # PATH/JAVA_HOME en el registro, broadcast, elevación UAC
└── commands\      # install, global, list, uninstall, current, setup, doctor, local
```

---

## Licencia

A tu elección (rellena antes de publicar).
