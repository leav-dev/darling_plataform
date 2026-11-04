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

| ID | Spec | Entrega | Commit |
| --- | --- | --- | --- |
| T1 | S3, S4 | ADR-0008 con el contrato verificado de Darling | `docs(adr)` |
| T2 | S5 | `PrefixLocation`, errores tipados y registro de estado (`phase`) con escritura atómica | `feat(shared-prefix)` |
| T3 | S2 | Locks de lectura/escritura sobre archivo de lock + pruebas de concurrencia | `feat(shared-prefix)` |
| T4 | S2, S3 | Adaptador `Darling` + `PrefixManager` (`inspect`, `validate`, `start`, `stop`) | `feat(shared-prefix)` |
| T5 | S2 | CLI `dg prefix status\|validate\|start\|stop` con salida texto y `--json` | `feat(cli)` |
| T6 | S2 | Documentación: mantenimiento, recuperación, límites de filesystem y mapa del repo | `docs(repo)` |

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
