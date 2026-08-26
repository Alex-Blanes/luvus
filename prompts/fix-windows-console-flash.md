# Arreglar el parpadeo de consolas en Windows (luvus v0.11.0)

Repo: `C:\Users\alex-\Documents\Proyectos\luvus` — Rust, rama `main`, limpio en `52357e6 release: v0.11.0`.
Diagnosticado el 2026-08-20 desde otra sesión. Nada implementado todavía: esto es el encargo completo.

## Síntoma

En Windows, con luvus abierto y 3 paneles, aparecen ventanas de consola negras que parpadean
sobre el escritorio ~45 veces por minuto, también mientras se escribe. Hace la herramienta
prácticamente inusable en Windows.

## Causa raíz (confirmada, no es hipótesis)

`luvus.exe server` ejecuta `git rev-parse --show-toplevel` cada ~2 segundos por workspace.
En Windows, un proceso hijo de subsistema consola lanzado desde otro proceso hace que el SO
le asigne un `conhost.exe` con ventana propia. `std::process::Command` **no** pasa
`CREATE_NO_WINDOW` (`0x0800_0000`) por defecto, así que cada llamada a `git` abre y cierra
una ventana.

Medido con este muestreo (útil también para verificar el arreglo — ejecutar con luvus abierto):

```powershell
$seen=@{}; $out=@()
1..80 | ForEach-Object {
  Get-CimInstance Win32_Process -Filter "Name='git.exe'" | ForEach-Object {
    $id=[int]$_.ProcessId
    if (-not $seen.ContainsKey($id)) {
      $seen[$id]=1
      $par = Get-CimInstance Win32_Process -Filter "ProcessId=$([int]$_.ParentProcessId)" -EA SilentlyContinue
      $out += '{0} | {1} | padre: {2}' -f $_.CreationDate.ToString('HH:mm:ss'), $_.CommandLine, $par.Name
    }
  }
  Start-Sleep -Milliseconds 200
}
$out
```

Antes del arreglo esto lista ~15 `git.exe` en 20 s, todos con padre `luvus.exe`.
Después debe listar los mismos procesos (siguen ejecutándose, es correcto) **sin** que aparezca
ninguna ventana en pantalla. El conteo de procesos NO debe bajar: si baja, se ha roto el polling
en vez de ocultar la ventana.

## El arreglo

El patrón ya existe en el repo, en `src/main.rs:514-526`:

```rust
#[cfg(windows)]
{
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0000_0008 | 0x0000_0200);
}
```

Falta aplicarlo a los spawns de consola. **Hazlo en un solo sitio compartido, no en cada
llamada**: añade un helper (p. ej. `fn no_window(cmd: &mut Command) -> &mut Command` en
`src/platform.rs`, que en no-Windows sea un no-op con `#[cfg]`) y encamina los spawns por ahí.

Sitios que lanzan `git` (13). El primero es el que cubre la mayoría:

- `src/git/local.rs:15` — helper `run()`, por el que pasan `is_repo`, `rev-parse --show-toplevel`
  (línea 865, el poller de 2 s), `--git-common-dir`, `--abbrev-ref HEAD`… **arreglar este solo
  ya quita el grueso del parpadeo**
- `src/git/local.rs`: 388, 587, 713, 757 — se saltan `run()`, hay que tocarlos uno a uno
- `src/app/board.rs`: 907, 1005, 1060
- `src/app/git.rs:1454`
- `src/app/modules.rs:1906`
- `src/main.rs:2262`
- `src/module/install.rs`: 221, 232

Otros binarios de consola que también parpadearán (mismo tratamiento, menos frecuentes):
`gh` en `src/git/github.rs` (3 sitios), `taskkill` en `src/terminal/pty.rs` (2),
y los spawns genéricos de `src/module/discovery.rs`, `src/skill.rs`, `src/update.rs`,
`src/cli.rs`, `src/platform.rs`.

**Cuidado:** no aplicar `CREATE_NO_WINDOW` a los spawns que crean paneles/PTY reales
(`src/terminal/pty.rs` para el shell del panel, `src/main.rs` para lanzar el agente) — esos
sí necesitan su consola. El flag va solo en los spawns de captura de salida (`.output()`,
`.status()`), no en los interactivos.

## Alcance y entrega

- Debe compilar en Windows, Linux y macOS: todo el código nuevo va bajo `#[cfg(windows)]`
  o con un no-op equivalente en el resto.
- Dejar una comprobación mínima: basta un test que confirme que el helper devuelve el flag
  esperado en Windows. No montar suite nueva.
- Sin dependencias nuevas: `std::os::windows::process::CommandExt` es de la stdlib, igual que
  el resto del repo.
- No commitear ni pushear sin que Alex lo pida.

## Contexto de la investigación (para no repetirla)

Se descartaron, en este orden, como causas del parpadeo:

1. **Hooks de Claude Code** — el parche de `pythonw` + `hookhide.py` sigue puesto, 65/65 en
   `hooks.json`. No eran.
2. **Servidores MCP** — 59 procesos entre 3 sesiones de Claude Code y Claude Desktop, pero
   ninguno huérfano y el parpadeo continuó con todos deshabilitados. Aparte de esto, hay
   `windows-mcp` declarado por triplicado (config de Desktop + extensión + global de Claude Code);
   es un despilfarro real de ~200 MB pero **no** la causa del parpadeo.
3. **Statusline propia** — sí tenía el mismo bug (`subprocess.run` sin `CREATE_NO_WINDOW`) y se
   arregló en `~/.local/bin/claude-statusline.py`, pero apenas se ejecuta: no era el emisor.

Emisor secundario, ajeno a luvus: **Docker Desktop** lanza `docker stats` ~30 veces por minuto
mientras su ventana de dashboard está abierta. Se quita cerrando esa ventana; el motor sigue vivo.
