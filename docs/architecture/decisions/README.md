# Architecture Decision Records

Un ADR registra **una** decisión arquitectónica significativa: el contexto que la
hizo necesaria, la decisión, las alternativas descartadas y sus consecuencias. No
es documentación de cómo funciona algo; para eso están
[`../overview.md`](../overview.md), [`../components.md`](../components.md) y
[`../data-flow.md`](../data-flow.md).

## Estados

| Estado | Significado |
| --- | --- |
| `propuesta` | En discusión; todavía no obliga. |
| `aceptada` | Vigente; el código y las revisiones se ajustan a ella. |
| `reemplazada` | Sustituida por un ADR posterior; se conserva como historia. |
| `rechazada` | Considerada y descartada. |

## Ciclo de vida

1. Copiá la estructura de un ADR existente y asigná el siguiente número.
2. Dejalo en `propuesta` mientras se discute en el issue o el PR.
3. Al integrarse, marcá `aceptada` y actualizá la tabla de abajo.
4. Si una decisión cambia: **no borres la anterior**. Marcá el ADR viejo como
   `reemplazada`, creá el nuevo y enlazalos en ambos sentidos.
5. Actualizá el plan y la documentación afectada en el mismo cambio.

## Índice

| ADR | Decisión | Estado |
| --- | --- | --- |
| [0001](ADR-0001-monorepo.md) | Monorepo como estrategia inicial | aceptada |
| [0002](ADR-0002-shared-dprefix.md) | Un DPREFIX compartido y dedicado | aceptada |
| [0003](ADR-0003-execution-daemon.md) | Daemon de ejecución por sesión de usuario | aceptada |
| [0004](ADR-0004-gpu-selection-policy.md) | Política Dedicated GPU First | aceptada |
| [0005](ADR-0005-dhi-separado-de-ejecucion.md) | DHI separado de las decisiones de ejecución | aceptada |
| [0006](ADR-0006-darling-nucleo-externo.md) | Darling como núcleo externo de compatibilidad | aceptada |
| [0007](ADR-0007-rust-y-cmake.md) | Rust para los módulos y CMake para el build | aceptada |
| [0008](ADR-0008-ciclo-de-vida-prefix.md) | Ciclo de vida verificado del DPREFIX de Darling | aceptada |
| [0009](ADR-0009-pin-de-darling.md) | Pin de Darling y verificación de instalación | aceptada |
| [0010](ADR-0010-instalador-deb.md) | Instalador de un comando entregado como paquete `.deb` | aceptada |
| [0011](ADR-0011-reporte-de-hardware.md) | Reporte de hardware desde procfs/sysfs con fuente inyectable | aceptada |
| [0012](ADR-0012-importacion-de-aplicaciones.md) | Importación de aplicaciones con staging, `.app`/`.zip` y catálogo SQLite | aceptada |
