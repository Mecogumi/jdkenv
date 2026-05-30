# jdkenv

A **JDK version manager for Windows**, native (PowerShell/cmd — no WSL, no Git
Bash). It downloads JDKs from the [foojay Disco API](https://api.foojay.io/) and
switches the active version instantly, without restarting your terminal.

Inspired by `pyenv`/`jenv`, written in Rust as a single dependency-free `.exe`
(TLS via rustls — no OpenSSL required).

---

## One-command install

```powershell
irm https://<MY_DOMAIN>/install.ps1 | iex
```

This downloads the right binary (x64/arm64) to `%USERPROFILE%\.jdkenv\bin\` and
runs `jdkenv setup` to register `PATH` and `JAVA_HOME`. Open a **new terminal**
and you're done.

> **Security note (honest):** `irm | iex` runs remote code without verification,
> exactly like `curl | bash`. It's the standard pattern used by Scoop/mise, but
> it means trusting the source (`<MY_DOMAIN>`). If you prefer, download the
> `.exe` from [Releases](https://github.com/<USER>/jdkenv/releases/latest), drop
> it at `%USERPROFILE%\.jdkenv\bin\jdkenv.exe`, and run `jdkenv setup`.

---

## Commands

| Command | What it does |
|---|---|
| `jdkenv install <version> [--distribution <dist>]` | Downloads and installs a JDK (`.zip`) from foojay. `--distribution` defaults to `temurin`. The first JDK you install is activated automatically. |
| `jdkenv global <version>` | Activates an installed version (re-points the `current` junction). |
| `jdkenv list` | Lists installed versions (`*` = active). |
| `jdkenv list --remote [--distribution <dist>]` | Lists versions available on foojay for Windows + your architecture. |
| `jdkenv uninstall <version>` | Deletes a version. Refuses if it's the active one. |
| `jdkenv current` (alias `which`) | Shows the active version and where `current` points. |
| `jdkenv setup [--system]` | Registers `PATH`/`JAVA_HOME`. Without a flag: user PATH. `--system`: system PATH (prompts for elevation). |
| `jdkenv setup --undo [--system]` | Reverts `setup`: removes jdkenv's PATH entries and `JAVA_HOME`. Does **not** delete installed JDKs. |
| `jdkenv doctor` | Diagnoses your environment and detects another `java.exe` that wins on PATH. |
| `jdkenv local <version>` | *(v2, not implemented yet)* Per-directory version. |

### Examples

```powershell
jdkenv install 21                          # Temurin 21 (latest build)
jdkenv install 17 --distribution corretto  # Corretto 17
jdkenv list --remote --distribution zulu   # what Zulu builds are available?
jdkenv global 17                            # switch the active JDK
jdkenv current                              # which one is active?
jdkenv doctor                               # is anything winning over me on PATH?
```

Versions accept prefixes: `21` resolves to the most recent build of that line
(e.g. `21.0.11+10`); you can also be specific (`17.0.13`). foojay distribution
names use underscores (e.g. `oracle_open_jdk`, `sap_machine`).

---

## The junction model (what makes switching instant)

Everything lives under `%USERPROFILE%\.jdkenv\`:

```
.jdkenv\
├── bin\
│   └── jdkenv.exe
├── versions\
│   ├── temurin-21.0.11+10\
│   └── corretto-17.0.13\
└── current\            ← NTFS junction → a folder inside versions\
```

The key idea: `PATH` and `JAVA_HOME` **always** point at `current`, never at a
specific version:

- `PATH` contains the literal path `...\.jdkenv\current\bin`
- `JAVA_HOME = ...\.jdkenv\current`

Switching versions (`jdkenv global <v>`) only **re-points** the `current`
junction → `versions\<dist>-<version>`. Because `current\bin` is a literal PATH
entry and `current` is a junction:

- **Already-open** terminals pick up the new version on the next `java` they
  launch — no restart, no re-broadcast needed.
- `JAVA_HOME` stays correct without touching the registry.

A **directory junction** is used on purpose (not a symlink): junctions **don't
require administrator rights** or Developer Mode. Re-pointing deletes the
junction (this does **not** delete the target — a junction is just a reparse
point) and recreates it pointing at the chosen version.

---

## The two PATHs: user vs system (important)

On Windows there are two PATHs, and the effective one is composed **SYSTEM
first, USER second**:

- **User** PATH → `HKCU\Environment`
- **System** PATH → `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment`

Consequence: prepending to the **user** PATH (what `jdkenv setup` does by
default) **does not beat** a `java.exe` that sits in the **system** PATH — the
classic case being Oracle's `javapath`, which the Oracle installer drops into
the system PATH.

- `jdkenv setup` (default) edits only `HKCU`: covers most cases without a UAC
  prompt.
- `jdkenv setup --system` edits `HKLM` and prepends there for **absolute
  priority**. It requires elevation; if you're not running as admin, jdkenv
  **relaunches itself elevated** (UAC) with the same arguments.
- `jdkenv doctor` tells you when you need `--system` (it detects which
  `java.exe` actually wins on your PATH).

> Many build tools (**Maven**, **Gradle**) prioritize `JAVA_HOME` over PATH.
> Since `setup` also sets `JAVA_HOME → current`, a lot of workflows just work
> even when the PATH order isn't perfect.

Windows implementation details:
- The registry is edited **directly**, not with `setx` (which truncates PATH at
  1024 characters).
- When rewriting `Path`, its value **type is preserved** (`REG_EXPAND_SZ`):
  downgrading it to `REG_SZ` would break existing references like `%SystemRoot%`.
- After writing, a `WM_SETTINGCHANGE` broadcast lets new terminals pick up the
  change without logging off.

---

## Undoing / uninstalling

`jdkenv setup --undo` reverses what `setup` did: it removes jdkenv's two PATH
entries and deletes `JAVA_HOME` — but only if `JAVA_HOME` still points at jdkenv
(it won't clobber a `JAVA_HOME` you set yourself). Add `--system` to undo a
`setup --system`. It is idempotent and does **not** remove installed JDKs or the
`current` junction.

To remove everything, including the installed JDKs:

```powershell
jdkenv setup --undo        # (and `jdkenv setup --undo --system` if you ran --system)
Remove-Item -Recurse -Force "$env:USERPROFILE\.jdkenv"
```

---

## Build from source

Requires [Rust](https://rustup.rs/) (MSVC toolchain).

```powershell
cargo build --release
# binary at target\release\jdkenv.exe

# Supported targets:
#   x86_64-pc-windows-msvc   (x64)
#   aarch64-pc-windows-msvc  (ARM64)
```

The project builds with no warnings and passes `cargo clippy`.

### Layout

```
src/
├── main.rs        # clap parser + dispatch
├── arch.rs        # x64/aarch64 detection → foojay parameter
├── paths.rs       # .jdkenv layout, junction (create/re-point), installed versions
├── foojay.rs      # Disco API client + .zip download/extraction
├── env_win.rs     # PATH/JAVA_HOME in the registry, broadcast, UAC elevation
└── commands\      # install, global, list, uninstall, current, setup, doctor, local
```

---

## License

Your choice (fill in before publishing).
