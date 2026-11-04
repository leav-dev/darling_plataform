# Feature: shared-prefix

Estado: en curso · Rama: `feat/shared-prefix` · Fase 2 del plan

El Shared Prefix Manager es la única autoridad de la plataforma sobre el DPREFIX
dedicado: ubicación, validación, ciclo de vida, locks y recuperación de estados
interrumpidos, más los comandos de CLI para consultarlo y validarlo.

La investigación de la interfaz real de Darling (ver S3) fijó qué es posible y qué
no: no existe comando de inicialización, el prefijo se crea on-demand, y no hay
forma documentada de consultar si el contenedor está vivo. El diseño se ajusta a
esos hechos en lugar de suponer una API que Darling no tiene.

## Specs

### S1 — Pedido original

> vamos por la fase 2

### S2 — Fase 2 del plan (verbatim)

> **Objetivo:** demostrar que la plataforma puede administrar un DPREFIX dedicado.
>
> Tareas: - Integrar una versión fijada de Darling. - Definir la ubicación del
> DPREFIX dedicado. - Implementar inicialización, validación, inicio y apagado. -
> Implementar locks de lectura/escritura. - Comprobar procesos activos antes de
> apagar. - Manejar estados interrumpidos y errores. - Documentar recuperación y
> límites del filesystem.
>
> Entregables: - Crate `shared-prefix`. - Pruebas de estado y concurrencia. -
> Herramienta CLI para consultar y validar el prefijo.
>
> **Criterio de salida:** el prefijo se inicializa y valida de forma repetible sin
> modificar el prefijo Darling convencional del usuario.

### S3 — Contrato verificado de Darling

Hechos comprobados en la documentación y el código de Darling, no supuestos:

| ID | Hecho | Fuente |
| --- | --- | --- |
| F1 | `DPREFIX` selecciona el prefijo; por defecto `~/.darling`. Debe estar **exportado** en el shell: usarlo solo al invocar Darling "no funciona como se espera". | `docs.darlinghq.org/darling-prefix.html` |
| F2 | **No hay comando de inicialización.** El prefijo se crea automáticamente en la primera ejecución (`checkPrefixDir()` → `setupPrefix()`). | `darling-prefix.html`, `src/startup/darling.c` |
| F3 | `darling shell [comando…]` arranca o se conecta al contenedor, inicializa el prefijo si hace falta y ejecuta el comando dentro. | `docs.darlinghq.org/darling-shell.html`, `darling.c` |
| F4 | `darling shutdown` termina el contenedor y **todos** los procesos que corren dentro. | `darling-prefix.html` |
| F5 | No hay comando documentado para consultar si el contenedor está vivo: la plataforma debe llevar su propio estado. | ausencia documentada |
| F6 | El prefijo es overlayfs sobre el root de Darling, dentro de un mount namespace de usuario; el filesystem superior debe soportar overlayfs. | `internals/basics/containerization.html`, `common-issues.html` |

### S4 — Regla del plan sobre supuestos

> No asumir comandos, interfaces internas ni comportamientos de Darling sin
> comprobarlos contra la versión objetivo.

### S5 — Restricciones de diseño del plan

> El prefijo será independiente del prefijo convencional del usuario, si existe. La
> ubicación propuesta es `~/.local/share/darling-platform/prefix/`.

> No se deben fabricar manualmente directorios internos que correspondan al sistema
> de Darling.

> No se debe detener el entorno mientras existan procesos gestionados activos.

## Tasks

| ID | Spec | Entrega | Commits |
| --- | --- | --- | --- |
| T1 | S3, S4 | ADR-0008 con el contrato verificado de Darling | `4875ffa` |
| T2 | S5 | `PrefixLocation`, errores tipados y registro de estado con escritura atómica | `194b880` |
| T3 | S2 | Locks de lectura/escritura sobre archivo de lock + pruebas de concurrencia | `56a86ef` |
| T4 | S2, S3 | Adaptador `Darling` + `PrefixManager` (`inspect`, `validate`, `start`, `stop`) | `cf17087` |
| T5 | S2 | CLI `dg prefix status\|validate\|start\|stop` con salida texto y `--json` | `45db3cb` |
| T6 | S2 | Documentación: recuperación, dependencias, switches de entorno y mapa del repo | `0613e0c` |

## Log

### L1 — Pedido original (verbatim)

> vamos por la fase 2

### L2 — Interpretación y alcance

"Fase 2" = Fase 2 del plan: integración mínima de Darling + Shared Prefix Manager.
La integración queda detrás de un adaptador (`Darling`), porque Darling **no está
instalado en la máquina de desarrollo**: el ciclo de vida se prueba con un doble y
el adaptador real queda aislado con sus comandos verificados por fuente (S3), a la
espera de la prueba local que exige la Fase 0.

Consecuencia explícita: este corte **no** permite todavía lanzar una app de macOS.
Desbloquea `dg prefix status|validate|start|stop` y la base sobre la que se apoyan
las fases 3 y 5.

### L3 — Decisiones de diseño derivadas de S3

- `start` = `darling shell /usr/bin/true`: es el único mecanismo verificado para
  asegurar que el contenedor está arriba y el prefijo creado (F2 + F3).
- `stop` = `darling shutdown`, guardado por el conteo de procesos gestionados
  (F4 + S5). En esta fase el conteo lo aporta el llamador; el catálogo de
  instancias llega con el DEM.
- No se consulta el estado del contenedor a Darling (F5): la plataforma persiste
  su propia fase y la marca `Error` cuando detecta una operación interrumpida.
- No se crean directorios internos del prefijo (S5): se ejecuta Darling y se deja
  que él lo estructure.

### L4 — Verificación (2026-11-04)

Árbol limpio, `scripts/ci/check.sh` (fmt, clippy `-D warnings`, 86 pruebas, CMake,
CTest) verde, más prueba de extremo a extremo con un `darling` falso:

| Comprobación | Resultado |
| --- | --- |
| `cargo fmt --all --check` | OK |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | OK, sin warnings |
| `cargo test --workspace --locked` | 86 pruebas, 0 fallos |
| `cmake --build build` + `ctest` | OK |
| `cmake --install --prefix /tmp/dp-install` | `dg` instalado; `prefix status --json` válido |
| E2E con `DG_DARLING` apuntando a un script | `start` → `started: running`; `stop` con 1 instancia → rechazo; `stop` → `stopped: stopped` |
| Argumentos realmente invocados | `shell /usr/bin/true` y `shutdown`, ambos con `DPREFIX=/tmp/dp-e2e/prefix` exportado |
| Estado persistido tras el ciclo | `{"schema_version":1,"phase":"stopped"}` |
| `prefix status` sin prefijo | no crea directorio, ni estado, ni lock (probado) |
| Sin Darling instalado | `start` falla con `DarlingUnavailable`, mensaje accionable y estado `error`/`start` |

### L5 — Desvíos registrados

- **Tamaño de los cortes.** La fase completa son ~4.100 líneas con pruebas. T4
  (adaptador + manager) y T5 (CLI) quedaron como commits únicos aunque superan el
  objetivo de ~400 líneas: partir un mismo archivo cohesivo habría producido
  commits intermedios que no compilan. Se registra acá como deuda de revisión; el
  orden de los commits sí es bisecable.
- **`CliEnv.darling` es `Arc<dyn Darling>`**, no `Box`: `execute` toma `&CliEnv` y
  `PrefixManager::new` necesita un adaptador propio `'static`. Se resuelve con un
  envoltorio local, sin cambiar la API del manager.
- **Sonda de lock sin efectos.** `PrefixLock::probe_busy` reemplaza al
  `try_exclusive` de `inspect`: un comando de lectura ya no crea el lock file ni el
  directorio de datos.
- **`darling_present` en `Inspection`** reemplaza el match sobre el texto de la
  advertencia que hacía frágil a la CLI.
- **uid del proceso** vía `rustix::process::getuid` (se habilitó el feature
  `process`), en lugar de inferirlo del dueño del lock file.
- **Sin `init`.** `start` es la única inicialización, porque Darling crea el
  prefijo on-demand (F2). No se agregó un comando `dg prefix init` que sugiera una
  operación que Darling no tiene.

### L6 — Pendiente fuera de este corte

- Prueba real contra un Darling instalado (`start`/`stop` en una máquina con
  Darling), prerequisito de la Fase 0 y único modo de cerrar F3/F4 localmente.
- Límite de overlayfs en el filesystem superior (`F6`): se documenta y se advierte,
  pero no se prueba con un montaje real.
- El conteo de instancias gestionadas lo aporta el llamador; el catálogo real llega
  con la Fase 3 y el daemon con la Fase 5.
