# Feature: hardware-intelligence (DHI, Fase 1)

Estado: en curso · Rama: `feat/hardware-intelligence` · Fase 1 del plan

Primer corte funcional del Hardware Intelligence Engine: `dp hardware scan` describe
el host (sistema, CPU, memoria, GPUs PCI y pantallas) con un reporte versionado y
diagnósticos por subsistema, sin root y sin decidir cómo se ejecutan aplicaciones.

## Specs

### S1 — Pedido original

> - Fase 1 — DHI (dp hardware scan): esqueleto en modules/hardware-intelligence/src/lib.rs vamos por esto

### S2 — Responsabilidades del DHI

`PLAN_DARLING_PLATFORM.md` §5.1, verbatim:

> - Arquitectura, modelo, topología y características de CPU.
> - Memoria del sistema.
> - Distribución, kernel y libc cuando pueda determinarse.
> - Enumeración de GPUs, incluyendo dispositivos PCI.
> - Información Vulkan real: dispositivos, extensiones, propiedades y features consultadas.
> - Información básica de sesión gráfica y pantallas cuando esté disponible.
> - Diagnósticos por subsistema y estado de descubrimiento.

La consulta Vulkan queda fuera de este corte: ver S8.

### S3 — Reglas del DHI

`PLAN_DARLING_PLATFORM.md` §5.1, verbatim:

> - Implementación principal en Rust.
> - No requerir privilegios elevados para el escaneo habitual.
> - No fallar globalmente por un proveedor individual.
> - No inferir features Vulkan a partir únicamente de la versión de API.
> - Enumerar GPUs por PCI/sysfs y correlacionarlas con Vulkan para reducir duplicados.
> - Mantener identificadores estables cuando sea posible, como PCI BDF.
> - Preparar proveedores específicos para x86-64 y AArch64.
> - No compilar con `-march=native` para distribuir binarios genéricos.

### S4 — Modelo conceptual

`PLAN_DARLING_PLATFORM.md` §5.1, verbatim:

> ``` rust
> pub struct HardwareReport {
>     pub schema_version: String,
>     pub scan_status: ScanStatus,
>     pub system: SystemInfo,
>     pub cpu: Option<CpuInfo>,
>     pub gpus: Vec<GpuInfo>,
>     pub memory: Option<MemoryInfo>,
>     pub displays: Vec<DisplayInfo>,
>     pub diagnostics: Vec<Diagnostic>,
> }
>
> pub enum ScanStatus {
>     Complete,
>     Partial,
>     Failed,
> }
>
> pub enum SupportLevel {
>     Supported,
>     Partial,
>     Unsupported,
>     Unknown,
> }
> ```

### S5 — Reporte versionado

`PLAN_DARLING_PLATFORM.md` §5.1, verbatim:

> El reporte JSON debe estar versionado para que el runtime pueda evolucionar sin romper consumidores.

### S6 — Entregables y criterio de salida de la Fase 1

`PLAN_DARLING_PLATFORM.md` §10, verbatim:

> Entregables: - CLI `dp-hardware scan`. - Salida JSON y texto. - Pruebas unitarias y fixtures. - Documentación de campos y limitaciones.

> **Criterio de salida:** el escaneo funciona sin root y la ausencia de Vulkan no invalida el resto del reporte.

### S7 — Nombre del comando

El plan escribe `dp-hardware scan`; el usuario y `docs/architecture/components.md`
usan **`dp hardware scan`**. Este corte implementa `dp hardware scan` y corrige la
mención del plan.

### S8 — Fuentes del corte 1

Decisión del usuario: **procfs/sysfs puro con fuente inyectable**, **sin Vulkan**.

- Lectura de `/etc/os-release`, `/proc`, `/sys` a través de un trait `HostSource`
  inyectable (`read`, `read_dir`, `exists`), para que los parsers se prueben con
  fixtures de texto en lugar de con el host real.
- La ausencia de cualquier fuente produce un `Diagnostic` y degrada el reporte;
  nunca lo invalida.
- Diagnostics debe declarar explícitamente que Vulkan no se consultó en este corte.
- Sin dependencias nuevas: sólo `serde` y `serde_json`, ya usadas en el workspace.

### S9 — Convenciones del repo

- CLI sin `clap`: argv a mano en `apps/cli/src/lib.rs`, `Response { code, stdout, stderr }`.
- Errores hand-rolled con `Display` accionable; sin `thiserror`.
- JSON con `serde_json::to_string` + `\n` final.
- Pruebas inline `#[cfg(test)] mod tests`, sin root, sin tocar el host real.

## Tasks

| ID | Spec | Entrega | Commit |
| --- | --- | --- | --- |
| T1 | S4, S5 | Modelo de dominio: `HardwareReport`, `ScanStatus`, `SupportLevel`, `SystemInfo`, `CpuInfo`, `MemoryInfo`, `GpuInfo`, `DisplayInfo`, `Diagnostic`, `schema_version` y serialización | `b38c9b8` |
| T2 | S2, S3, S8 | `HostSource` inyectable + `LinuxHost` + proveedores de sistema, CPU y memoria con diagnósticos | `994b389` |
| T3 | S2, S3, S8 | Enumeración de GPUs PCI por sysfs (BDF estable) y pantallas/sesión gráfica | `dcb3152` |
| T4 | S6, S7, S9 | CLI `dp hardware scan` con salida texto y `--json`, help y códigos de salida | `aa5f994` |
| T5 | S6 | Docs de campos y limitaciones, `components.md`, `data-flow.md`, ADR-0011 y CHANGELOG | `7f716ec` |

## Log

- **L1** — Pedido del usuario, verbatim: «- Fase 1 — DHI (dp hardware scan): esqueleto en modules/hardware-intelligence/src/lib.rs vamos por esto»
- **L2** — Relevamiento: `modules/hardware-intelligence` sólo tiene el doc-comment y un test trivial; no existe ningún tipo del reporte. `docs/architecture/components.md:222` ya anuncia `dp hardware scan`; el plan dice `dp-hardware scan`. Sin `clap`, sin `[workspace.dependencies]`, deps por crate. ADR libre: 0011.
- **L3** — Decisión del usuario: Vulkan fuera del corte 1. Motivo: mantener cero dependencias nuevas y un reporte verificable en esta máquina y en CI headless; Vulkan entra como tarea propia con ADR y loader dinámico.
- **L4** — Decisión del usuario: procfs/sysfs puro con fuente inyectable. Motivo: parsers testeables con fixtures y crate portable; `raw-cpuid` queda descartado para este corte.
- **L5** — Relevamiento de entorno: el workspace no compila en Windows (`dp-shared-prefix` usa APIs Unix); el toolchain MSVC está roto (`link.exe` resuelto al `link` de Git). El crate DHI se mantiene std-only para poder correr `cargo test -p dp-hardware-intelligence` con el toolchain GNU.
- **L6** — Verificación local: 61 tests de `dp-hardware-intelligence` en verde, `clippy -D warnings` y `rustfmt --check` limpios. `dp-cli` no se puede compilar en esta máquina (depende de `dp-shared-prefix`): sus seis tests nuevos y el comando quedan verficados por revisión estática y por CI en Linux. Una verificación independiente detectó que faltaban los re-exports de `HostSource`, `LinuxHost` y `MappingSource`; corregido antes del commit.
