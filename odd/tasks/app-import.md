# Feature: app-import (Bundle Inspector, App Installer y catálogo, Fase 3)

Estado: en curso · Rama: `feat/app-import` (sobre `feat/hardware-intelligence`) · Fase 3 del plan

Importar y registrar una aplicación macOS sin ejecutarla: inspeccionar el bundle,
extraerlo en staging, validarlo, instalarlo en el DPREFIX compartido y dejarlo en un
catálogo consultable. DMG, PKG y rollback quedan fuera de este corte.

## Specs

### S1 — Pedido original

> - Fase 3 — Bundle Inspector / App Installer: esqueletos

### S2 — Bundle Inspector

`PLAN_DARLING_PLATFORM.md` §5.5, verbatim:

> Debe extraer metadatos sin ejecutar el binario:
>
> -   `CFBundleIdentifier`
> -   `CFBundleName`
> -   `CFBundleDisplayName`
> -   `CFBundleVersion`
> -   `CFBundleExecutable`
> -   `CFBundleIconFile`
> -   `LSMinimumSystemVersion`
> -   Arquitecturas y slices Mach-O.
> -   Rutas de frameworks y recursos del bundle.
>
> Debe validar que los paths declarados permanezcan dentro del bundle y
> que el ejecutable esperado exista.

### S3 — Darling App Installer

`PLAN_DARLING_PLATFORM.md` §5.4, verbatim (reglas):

> - Extraer primero en staging.
> - Validar rutas antes de copiar archivos.
> - No sobrescribir una instalación existente sin política explícita.
> - Registrar el origen, versión y resultado de la instalación.
> - Para bundles copiados o ZIP, diseñar una estrategia de rollback.
> - Para PKG, advertir que scripts y efectos externos pueden impedir un rollback completo.
> - No ejecutar scripts de instalación de forma silenciosa.
> - Convertir iconos `.icns` a un formato adecuado para la interfaz, conservando el original cuando sea útil.

Y el orden de formatos, verbatim:

> Formatos por prioridad: 1. `.app` (bundle ya extraído). 2. `.zip`. 3. `.dmg`. 4. `.pkg`, con advertencias y consentimiento explícito. 5. `.xip`, después de investigar soporte y herramientas adecuadas.

### S4 — Application Library

`PLAN_DARLING_PLATFORM.md` §5.6, verbatim:

> Almacenamiento inicial: - SQLite. - Datos fuera del DPREFIX. - Iconos y cachés separados. - Migraciones de esquema versionadas.

Tabla `applications` orientativa del plan: `id`, `bundle_id`, `name`, `version`,
`bundle_path`, `executable_path`, `icon_path`, `architecture`,
`installation_type`, `installed_at`, `installation_status`.

### S5 — Tareas, entregables y criterio de salida de la Fase 3

`PLAN_DARLING_PLATFORM.md` §10, verbatim:

> Tareas: - Inspeccionar `Info.plist`. - Detectar ejecutable, icono y metadatos. - Analizar arquitectura Mach-O. - Implementar staging seguro. - Incorporar primero bundles `.app` y ZIP. - Añadir DMG después de verificar el flujo de Darling. - Tratar PKG como flujo explícito y de mayor riesgo. - Implementar SQLite y migraciones. - Registrar historial de instalación. - Añadir rollback donde sea posible.

> Entregables: - Inspector CLI. - Instalador de primer formato soportado. - Catálogo SQLite. - Tests con bundles de ejemplo y archivos malformados.

> **Criterio de salida:** una aplicación se importa, inspecciona, instala y registra de forma repetible; los fallos no dejan instalaciones parciales silenciosas.

### S6 — Alcance de este corte

Decisión del usuario:

- Dentro: inspector, `.app` y `.zip`, catálogo SQLite con migraciones, historial de
  instalación, CLI `dp app inspect|install|list`.
- Fuera: DMG, PKG, `.xip` (formato y flujo propios, con su ADR), rollback
  implementado (se diseña en el ADR y se implementa en un corte siguiente), y
  conversión de iconos `.icns` (no hay todavía interfaz que los consuma; el
  inspector reporta el icono declarado y preserva el original).

### S7 — Dependencias y destinos

Decisión del usuario:

- `plist` para `Info.plist` (XML y binario), `zip` para comprimidos, `rusqlite` con
  la feature `bundled` para el catálogo: ninguna dependencia de sistema, así el
  `.deb` no gana un `Depends` nuevo.
- Bundles instalados en `<DPREFIX>/Applications/<Bundle>.app`; catálogo e iconos en
  el data home de la plataforma (`~/.local/share/darling-platform/`), fuera del
  DPREFIX.
- La instalación toma el lock de escritura del Shared Prefix Manager y falla si hay
  instancias activas.

### S8 — Convenciones del repo

- CLI sin `clap`, `Response { code, stdout, stderr }`, exit 2 para uso, 1 para fallo.
- Errores hand-rolled con `Display` accionable.
- Pruebas inline `#[cfg(test)] mod tests`, con fixtures, sin root.
- `Info.plist`, Mach-O, ZIP y rutas del bundle son entrada no confiable: se valida
  antes de copiar y se rechaza traversal, symlinks que escapan y escrituras fuera
  del destino.

## Tasks

| ID | Spec | Entrega | Commit |
| --- | --- | --- | --- |
| T1 | S2, S5 | Bundle Inspector: `Info.plist` (XML y binario), metadata, validación de paths dentro del bundle, ejecutable existente y arquitecturas Mach-O | — |
| T2 | S4, S5 | Application Library: esquema SQLite versionado, migraciones, alta y consulta de aplicaciones e historial | — |
| T3 | S3, S5, S6, S7, S8 | App Installer: staging para `.app` y `.zip`, extracción segura, validación con el inspector, instalación atómica en el DPREFIX con lock, política de sobrescritura e historial | — |
| T4 | S5, S6 | CLI `dp app inspect\|install\|list` con texto y `--json` | — |
| T5 | S5, S6 | Documentación: campos y límites, `components.md`, `data-flow.md`, ADR-0012 (formatos, staging, rollback y riesgos de PKG) y CHANGELOG | — |

## Log

- **L1** — Pedido del usuario, verbatim: «- Fase 3 — Bundle Inspector / App Installer: esqueletos»
- **L2** — Relevamiento: los tres crates (`bundle-inspector`, `app-installer`, `app-library`) sólo tienen el doc-comment y un test trivial; ese texto es el contrato a respetar. El CLI ya tiene convenciones fijas (sin `clap`, `Response`, JSON compacto + `\n`). ADR libre: 0012.
- **L3** — Decisión del usuario: corte 1 = inspector + `.app`/`.zip` + SQLite + historial; DMG, PKG, `.xip` y rollback quedan para un corte siguiente.
- **L4** — Decisión del usuario: `plist` + `zip` + `rusqlite` con `bundled`; nada de dependencias de sistema nuevas en el `.deb`.
- **L5** — Decisión del usuario: bundles en `<DPREFIX>/Applications/<Bundle>.app`, catálogo e iconos en el data home, con lock de escritura del SPM durante la instalación.
- **L6** — Decisión de alcance: la conversión de `.icns` se difiere hasta que exista una interfaz que consuma iconos; el inspector reporta el icono declarado y el instalador preserva el original. Riesgo declarado del corte: escritura de archivos y extracción de comprimidos de entrada no confiable (ítems 1 y 2 de riesgo alto), así que la verificación incluye revisión independiente.
