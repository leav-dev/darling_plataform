# Feature: compatibility-engine (Fase 4)

Estado: en curso · Rama: `feat/compatibility-engine` (sobre `feat/app-import`) · Fase 4 del plan

Analiza estáticamente los requisitos de una aplicación macOS y los compara con un
perfil versionado de las capacidades de Darling, para decir si es compatible,
parcialmente compatible, no compatible o desconocida — con razones y evidencias.

## Specs

### S1 — Pedido original

> Fase 4 — Compatibility Engine: esqueleto

### S2 — Objetivo y análisis estático inicial

`PLAN_DARLING_PLATFORM.md` §5.7, verbatim:

> **Objetivo:** analizar estática y dinámicamente, cuando sea posible, los
> requisitos de una aplicación y compararlos con las capacidades conocidas
> de Darling y del host.
>
> Análisis estático inicial: - Arquitectura Mach-O y slices. - Tipo de
> binario y ejecutable principal. - Dependencias dinámicas (`dylib`). -
> Frameworks referenciados. - Versión mínima de macOS declarada. -
> Metadatos del bundle. - Recursos y componentes requeridos
> identificables.

### S3 — Lo que el motor debe producir

`PLAN_DARLING_PLATFORM.md` §5.7, verbatim:

> El motor debe producir: - Resultado general. - Evidencias encontradas. -
> Requisitos satisfechos. - Requisitos no satisfechos. - Elementos
> desconocidos. - Advertencias y recomendaciones. - Versión del analizador
> y del perfil de capacidades utilizado.
>
> No convertir un análisis estático favorable en una garantía de
> ejecución. La compatibilidad real requiere pruebas.

### S4 — Fase 4: tareas, entregables y criterio de salida

`PLAN_DARLING_PLATFORM.md` §10, verbatim:

> Tareas: - Inspeccionar slices y arquitectura Mach-O. - Leer dependencias
> dinámicas. - Identificar frameworks requeridos. - Leer versión mínima
> declarada de macOS. - Comparar requisitos con un perfil versionado de
> capacidades de Darling. - Reportar desconocidos y limitaciones. - Diseñar
> el formato de evidencia y diagnóstico.

> Entregables: - CLI de análisis. - Reporte de compatibilidad
> versionado. - Matriz inicial de aplicaciones de prueba.

> **Criterio de salida:** el sistema diferencia claramente compatible,
> parcialmente compatible, no compatible y desconocido, con razones y
> evidencias.

### S5 — Contrato del crate

`modules/compatibility-engine/src/lib.rs`, verbatim:

> Responsibility: analyse an application's requirements — Mach-O slices,
> dynamic dependencies, referenced frameworks, declared minimum macOS version,
> bundle metadata — and compare them with a versioned profile of the
> capabilities of the pinned Darling build and the detected host.
>
> Not responsible for: executing the application to inspect it, mutating the
> prefix, or promising compatibility. A favourable static analysis is evidence,
> never a guarantee: the report must state what was checked, what failed and
> what remains unknown, together with the analyser and capability-profile
> versions it used.

### S6 — Perfil de capacidades

Decisión del usuario: archivo versionado `deps/darling-capabilities.toml`,
embebido en el binario con `include_str!`, para que el reporte cite exactamente
la versión que se revisa en el repositorio. Contenido mínimo: `profile_version`,
`darwin_version`, y entradas de `frameworks` y `dylibs` con su estado
(`supported`, `partial`, `unsupported`, `unknown`).

### S7 — Veredicto por severidad

Decisión del usuario: el resultado general se deriva del peor requisito —
`not_compatible` si algo está definitivamente insatisfecho, `unknown` si nada
falla pero algo no se puede decidir, `partial` si todo se satisface con
advertencias, `compatible` sólo si todo se comprobó y se satisfizo. Un
`unknown` nunca se presenta como compatible ni como fallo.

### S8 — Alcance de este corte

Decisión del usuario:

- Dentro: arquitecturas vs host, dependencias dinámicas y frameworks vs perfil,
  versión mínima de macOS vs versión Darwin del perfil, reporte versionado con
  evidencias, CLI `dp app compat <ruta> [--json]`.
- Fuera: gating de `dp app install` (el análisis informa, no bloquea), análisis
  de símbolos importados, requisitos gráficos/audio/entrada, DRM/anticheat,
  análisis dinámico, matriz de aplicaciones reales (necesita bundles de prueba
  que el repo no puede versionar).
- La arquitectura del host la provee el llamador (`std::env::consts::ARCH` en el
  CLI), así el motor es determinista y testeable.

## Tasks

| ID | Spec | Entrega | Commit |
| --- | --- | --- | --- |
| T1 | S2, S4 | Mach-O: dependencias dinámicas y versión mínima del OS desde los load commands, expuestas por `dp-bundle-inspector` sin romper su contrato | — |
| T2 | S6 | Perfil de capacidades: `deps/darling-capabilities.toml`, modelo y parser con validación, embebido y versionado | — |
| T3 | S3, S4, S5, S7 | Motor de análisis: requisitos, evidencias, veredicto por severidad y reporte versionado | — |
| T4 | S4, S8 | CLI `dp app compat <ruta> [--json]` con texto y JSON | — |
| T5 | S3, S4 | Documentación: campos y límites, `components.md`, `data-flow.md`, ADR-0013 y CHANGELOG | — |

## Log

- **L1** — Pedido del usuario, verbatim: «Fase 4 — Compatibility Engine: esqueleto»
- **L2** — Relevamiento: `dp-compatibility-engine` sólo tiene doc-comment; el inspector ya parsea headers Mach-O (thin y fat) y expone arquitecturas; el CLI ya tiene `dp app inspect|install|list` y `dp hardware scan`. ADR libre: 0013.
- **L3** — Decisión del usuario: perfil TOML versionado en `deps/darling-capabilities.toml`, embebido con `include_str!`, con la dependencia `toml`.
- **L4** — Decisión del usuario: veredicto por severidad del peor requisito (insatisfecho > desconocido > advertencia > compatible).
- **L5** — Decisión del usuario: este corte sólo analiza; `dp app install` no cambia y nada queda bloqueado por un perfil inicial incompleto.
- **L6** — Riesgo declarado: el corte lee headers Mach-O de entrada no confiable (ítem 2) y produce un veredicto que un operador puede leer como garantía; la documentación debe repetir que el análisis estático es evidencia y no promesa.
