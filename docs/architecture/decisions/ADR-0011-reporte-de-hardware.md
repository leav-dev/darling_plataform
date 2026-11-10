# ADR-0011: Reporte de hardware desde procfs/sysfs con fuente inyectable

- **Estado:** aceptada
- **Fecha:** 2026-11-09
- **Responsables:** mantenedores de Darling Platform
- **Reemplaza a:** —
- **Reemplazada por:** —

## Contexto

La Fase 1 del plan pide un inventario de hardware útil y un reporte JSON
versionado, con diagnósticos por subsistema y criterio de salida explícito: el
escaneo funciona sin root y la ausencia de Vulkan no invalida el resto del
reporte.

El plan también enumera fuentes concretas: `raw-cpuid` para CPU y el loader de
Vulkan (con `ash`) para las capacidades gráficas reales. Esas dos elecciones
traen costo antes de traer valor:

- `raw-cpuid` es específico de x86-64; el plan exige proveedores para x86-64 y
  AArch64, y hoy la mitad del trabajo sería un `cfg` distinto por arquitectura.
- La enumeración Vulkan depende de hardware real y de un loader presente. En un
  CI headless no hay GPU, no hay ICD y no hay nada que probar, así que esa parte
  llegaría sin pruebas posibles.
- Los parsers que leen el host directamente desde `std::fs` no se pueden probar
  salvo describiendo la máquina que corre los tests: resultados distintos por
  desarrollador y ninguna prueba reproducible en CI.

El repo, además, tiene convenciones fijas: CLI sin `clap`, errores hand-rolled,
JSON con `serde_json`, pruebas inline sin tocar el host real.

## Decisión

1. **Se describe el hardware con procfs/sysfs puro.** El reporte se construye
   leyendo `/etc/os-release`, `/proc/cpuinfo`, `/proc/meminfo`,
   `/proc/sys/kernel/osrelease`, `/sys/bus/pci/devices` y `/sys/class/drm`. Son
   interfaces estables del kernel, están en todas las distribuciones Linux y no
   requieren privilegios.
2. **Todo acceso al host pasa por un trait inyectable `HostSource`** (`read`,
   `read_dir`, `exists`, `read_link`) con dos implementaciones: `LinuxHost` para el
   host real y `MappingSource` para un host capturado en memoria. El CLI inyecta
   la fuente en `CliEnv` y la pasa al escaneo; los proveedores nunca llaman a
   `std::fs`. Los parsers se prueban con texto de fixtures.
3. **Sin dependencias nuevas.** El crate usa solo `std` más `serde` (derive), ya
   presente en el workspace; `serde_json` aparece únicamente como
   dev-dependency y en el CLI. Nada de FFI, nada de crates por arquitectura.
4. **Vulkan se difiere a su propio corte.** Este escaneo no carga el loader, no
   enumera dispositivos ni infiere extensiones. El reporte lo declara con el
   diagnóstico informativo `not_consulted` (proveedor `vulkan`, nivel `info`), de
   modo que la ausencia de sección Vulkan es una afirmación explícita y no un
   silencio ambiguo.
5. **El reporte está versionado y lleva diagnósticos por subsistema.**
   `schema_version = "1.0"`; los proveedores no se abortan entre sí y cada
   degradación queda en `diagnostics` con proveedor, nivel y código estable. El
   nivel agregado vive en `scan_status` (`complete`, `partial`, `failed`).
6. **Sin root y sin escrituras.** El escaneo solo lee rutas absolutas y resuelve
   enlaces de sysfs. No crea archivos, no ejecuta comandos del host, no carga
   módulos y no cambia configuración.

## Alternativas consideradas

| Alternativa | Por qué no |
| --- | --- |
| `raw-cpuid` para la CPU, como pide el plan | Solo x86-64; el plan exige también AArch64. Y lee CPUID, no lo que el kernel publica, así que difiere del resto de las fuentes del reporte |
| `ash` más el loader de Vulkan en este corte | Requiere GPU e ICD reales: imposible de probar en CI headless, y arrastra una dependencia grande antes de que exista consumidor |
| Parsear la línea de comandos con `clap` | El repo tiene CLI hand-rolled por convención (ADR y estilo del `dp` actual); cambiarlo es una decisión aparte, no un efecto colateral del DHI |
| Leer el host con `std::fs` dentro de cada proveedor | Pruebas no reproducibles: el resultado depende de la máquina. La fuente inyectable es lo que permite fixtures y CI |
| Incluir una sección Vulkan vacía o con `null` | Un consumidor no podría distinguir "no se consultó" de "no hay Vulkan". El diagnóstico explícito elimina la ambigüedad |
| Fallar el escaneo cuando falta un subsistema | Contradice la regla del plan: un proveedor individual no puede invalidar el reporte |

## Consecuencias

### Positivas

- Los parsers son deterministas y se prueban con fixtures de texto, sin GPU, sin
  root y sin depender del host de CI.
- El crate compila para cualquier arquitectura Linux sin `cfg` por familia de CPU.
- Cero dependencias nuevas: nada que auditar, nada que vuelva a romper el build.
- La ausencia de Vulkan es explícita y verificable en el reporte, así que el
  criterio de salida de la Fase 1 se cumple por construcción.
- El contrato de serialización queda fijado por pruebas: enums en `snake_case`,
  ausencias omitidas en lugar de `null`, colecciones siempre presentes.

### Negativas y riesgos

- **La descripción depende de lo que el kernel publique.** En hosts donde falta
  un campo (por ejemplo `physical id` en AArch64) la topología queda ausente; en un
  contenedor sin sysfs montado no hay GPUs que enumerar. El reporte lo diagnostica,
  pero no puede inventarlo.
- **La familia de libc es una heurística** sobre rutas de loader y objetos
  compartidos. Publica la ruta de evidencia (`libc.evidence`) para que la
  afirmación sea revisable, y no promete soporte a partir de ella.
- **`raw-cpuid` habría dado datos que `/proc/cpuinfo` no publica** (por ejemplo
  detalles de CPUID por familia). Si una fase posterior los necesita, hay que
  reintroducir la dependencia con su propio ADR.
- **La ausencia de Vulkan bloquea cualquier política gráfica real.** Mientras dure
  el diferimiento, el runtime no puede elegir backend Vulkan ni correlacionar
  dispositivos: solo conoce PCI.
- **El nivel de un diagnóstico decide `scan_status`.** Un `warning` degrada el
  reporte a `partial`; si un proveedor empieza a emitir advertencias por casos
  benignos, el estado se vuelve ruidoso. La disciplina de niveles es parte del
  contrato.

## Seguimiento

- Corte propio para Vulkan: loader dinámico, dispositivos, extensiones,
  propiedades y features consultadas, más la correlación con PCI BDF, con su ADR.
- Reevaluar `raw-cpuid` si aparece un requisito que `/proc/cpuinfo` no cubra, y
  documentar el costo por arquitectura.
- Revisar la heurística de libc si el reporte se usa para decidir compatibilidad:
  hoy describe, no decide.
- Agregar fixtures de hosts reales (contenedor sin sysfs, host headless, doble GPU)
  a medida que aparezcan casos que el reporte describa mal.
