# Componentes

Un apartado por componente: qué hace, qué no hace, qué contrato expone y en qué
estado está hoy. Los contratos son **conceptuales**: nombres y campos pueden
cambiar hasta que exista implementación y pruebas que los fijen.

Estado actual: hay componentes implementados (hardware, prefijo, inspector,
instalador, catálogo y motor de compatibilidad) y componentes que siguen siendo
esqueleto: declaran su límite en la documentación del crate y todavía no tienen
lógica.

---

## Hardware Intelligence Engine (DHI)

**Crate:** `dp-hardware-intelligence` · **Estado:** implementado (Fase 1)

Produce un perfil verificable del host: arquitectura, modelo y features de CPU,
memoria, distribución, kernel, libc, GPUs por PCI/sysfs y pantallas que publica el
kernel. Vulkan queda fuera de este corte (ver más abajo).

- **Hace:** detecta, normaliza y publica un reporte versionado con diagnósticos
  por subsistema, desde `dp hardware scan` en texto o `--json`.
- **No hace:** instalar drivers, modificar el host, elegir GPU o backend, ni
  consultar Vulkan todavía.
- **Reglas:** sin root; tolerante a fallos (un proveedor que falla degrada el
  reporte a `partial`, no lo invalida); identificadores estables (PCI BDF); sin
  `-march=native`; sin dependencias nuevas (`serde` ya estaba en el workspace).

```rust
pub struct HardwareReport {
    pub schema_version: String,  // "1.0"
    pub scan_status: ScanStatus, // Complete | Partial | Failed
    pub system: SystemInfo,
    pub cpu: Option<CpuInfo>,
    pub gpus: Vec<GpuInfo>,
    pub memory: Option<MemoryInfo>,
    pub displays: Vec<DisplayInfo>,
    pub diagnostics: Vec<Diagnostic>,
}
```

El reporte se versiona para que el runtime evolucione sin romper consumidores:
agregar campos mantiene `1.0`, quitar o resignificar un campo sube el mayor.
Campos, reglas de serialización, códigos de diagnóstico y limitaciones están en
[`hardware-report.md`](hardware-report.md).

Implementación actual: un trait `HostSource` inyectable (`read`, `read_dir`,
`exists`, `read_link`) con dos implementaciones, `LinuxHost` para el host real y
`MappingSource` para reproducir un host capturado en pruebas, y cinco proveedores
que nunca abortan entre sí: `system` (`/etc/os-release`, release del kernel, libc
por heurística sobre el loader), `cpu` (`/proc/cpuinfo`), `memory`
(`/proc/meminfo`), `gpu` (clase PCI `0x03` en sysfs, BDF como identificador) y
`display` (conectores de `/sys/class/drm`). El CLI inyecta el `HostSource` en
`CliEnv`, igual que inyecta el entorno del prefijo y el adaptador de Darling.

**No hace todavía:** enumerar Vulkan. No hay dispositivos, extensiones,
propiedades ni features Vulkan en el reporte, y el escaneo lo declara con el
diagnóstico informativo `not_consulted` para que nadie lea el silencio como
evidencia. La correlación PCI con Vulkan tampoco existe aún: sin lado Vulkan no
hay nada que correlacionar.

---

## Shared Prefix Manager (SPM)

**Crate:** `dp-shared-prefix` · **Estado:** implementado (Fase 2)

Única autoridad sobre el DPREFIX dedicado y compartido
(`~/.local/share/darling-platform/prefix/`).

- **Hace:** inicializa mediante los mecanismos soportados por Darling, valida la
  estructura, inicia y detiene el entorno, mantiene locks y se recupera de
  operaciones interrumpidas.
- **No hace:** fabricar directorios internos de Darling, reimplementar
  `darlingserver` o `launchd`, ni guardar datos de la plataforma dentro del
  prefijo.
- **Política de concurrencia:** lecturas y consultas conviven con aplicaciones en
  ejecución; instalación, actualización y desinstalación toman lock de escritura;
  el apagado exige que no queden procesos gestionados activos.

```rust
pub enum PrefixState {
    NotInitialized, Initializing, Stopped, Starting,
    Running, Busy, Stopping, Error,
}
```

Son estados de la plataforma, no estados internos oficiales de Darling, y no deben
presentarse como tales.

**Verificado en esta fase** (ver `ADR-0008`): Darling no tiene comando de
inicialización —el prefijo se crea en la primera ejecución—, `darling shell
/usr/bin/true` arranca o se conecta al contenedor, `darling shutdown` termina
todos los procesos que corren dentro, y no hay forma documentada de consultar si
el contenedor está vivo. Por eso el estado lo persiste la plataforma
(`state.json`, escritura atómica) y una fase que quedó a mitad de transición se
reporta como interrumpida.

Implementación actual:

| Operación | Comando | Efecto |
| --- | --- | --- |
| Consulta | `dp prefix status [--json]` | Reporta ubicación, fase, interrupción, lock ocupado y advertencias; no escribe nada |
| Validación | `dp prefix validate [--json]` | Diez checks: existencia, **directorio padre**, tipo, symlink, propiedad, escritura, longitud, ubicación dedicada, datos fuera del prefijo y registro de estado |
| Arranque | `dp prefix start` | Crea el **directorio padre** del prefijo, luego `darling shell /usr/bin/true` con `DPREFIX` exportado; persiste `Initializing`/`Starting` antes de invocar |
| Apagado | `dp prefix stop [--active-instances N]` | Se niega con instancias activas; si no, `darling shutdown` |

Locks: lecturas concurrentes con lock compartido, operaciones de cambio con lock
exclusivo no bloqueante (`PrefixBusy` en lugar de esperar). El lock vive **fuera**
del prefijo, para que no lo oculte el overlay.

---

## Darling Execution Manager (DEM)

**Crate:** `dp-execution-manager` · **Estado:** esqueleto

Daemon por sesión de usuario, activado bajo demanda (D-Bus de sesión y
`systemd --user` cuando estén disponibles).

Secuencia de un lanzamiento: resolver el `app_id` en el catálogo → verificar
bundle y ejecutable → consultar el análisis de compatibilidad y advertir riesgos →
asegurar el prefijo en ejecución → preparar el entorno → lanzar por una interfaz
Darling verificada → supervisar el proceso → notificar eventos → mantener el
prefijo vivo mientras haya aplicaciones gestionadas.

- **No hace:** reimplementar componentes de Darling ni aceptar comandos de shell
  arbitrarios.
- **Contrato:** métodos `LaunchApp`, `StopApp`, `GetAppStatus`, `ListRunningApps`,
  `GetPrefixStatus`; eventos `AppStarted`, `AppStopped`, `AppCrashed`,
  `PrefixStateChanged`. Los nombres son propuesta hasta que exista la interfaz
  estable.

```rust
pub struct LaunchRequest { pub app_id: String, pub arguments: Vec<String> }

pub struct RunningApplication {
    pub instance_id: String,
    pub app_id: String,
    pub host_pid: Option<u32>,   // PID del proceso Linux, no del entorno Darwin
    pub started_at: String,
    pub status: ApplicationStatus, // Starting | Running | Stopping | Stopped | Crashed | Unknown
}
```

---

## Darling App Installer (DAI)

**Crate:** `dp-app-installer` · **Estado:** implementado (Fase 3, corte 1) para
`.app` y `.zip`

Importa un bundle en el DPREFIX compartido sin ejecutarlo. Flujo, en orden:
identificar formato por extensión → crear
`<prefijo>/Applications/.staging/<installation_id>` → copiar el `.app` o extraer
el `.zip` validando cada path mientras se escribe → encontrar el único `.app`
preparado → inspeccionarlo con el Bundle Inspector → derivar
`<prefijo>/Applications/<nombre>.app` → rechazar un destino ocupado salvo
`--replace` → un solo `std::fs::rename` → escribir las dos filas del catálogo en
una transacción → borrar el staging.

- **Hace:** staging en el mismo sistema de archivos que el destino, extracción
  segura del ZIP (200 000 entradas y 8 GiB descomprimidos como máximos), copia
  del `.app` con enlaces simbólicos recreados solo si su destino canónico sigue
  dentro del bundle, política de reemplazo explícita y registro de cada intento,
  con éxito o sin él.
- **No hace:** declarar compatibilidad, ejecutar nada, resolver el prefijo ni
  tomar el lock (lo hace el CLI antes de llamarlo), ni sobrescribir una
  instalación existente sin política explícita.
- **Riesgo:** los archivos son entrada no confiable. Ver las reglas completas en
  [`application-install.md`](application-install.md).

**No hace todavía:** `.dmg`, `.pkg` y `.xip` (se rechazan nombrando la ruta y los
formatos soportados), rollback de una instalación completada y conversión de
iconos `.icns`.

---

## Bundle Inspector

**Crate:** `dp-bundle-inspector` · **Estado:** implementado (Fase 3, corte 1)

Extrae metadatos **sin ejecutar** el binario: `CFBundleIdentifier`,
`CFBundleName` (con el nombre del directorio como respaldo, igual que macOS),
`CFBundleDisplayName`, `CFBundleVersion`, `CFBundleExecutable`,
`CFBundleIconFile`, `LSMinimumSystemVersion`, arquitecturas y slices Mach-O, y
rutas de frameworks y recursos.

- **Hace:** lee `Contents/Info.plist` en XML o binario; valida cada ruta
  declarada léxicamente (nada absoluto, ningún `..`) y físicamente (ningún
  symlink que salga del bundle); exige que el ejecutable declarado exista, sea
  archivo regular y tenga cabecera Mach-O; reporta las arquitecturas por slice
  (archivo delgado o universal) sin consultar `file`, `otool` ni `lipo`. Se
  expone con `dp app inspect <path> [--json]`.
- **No hace:** ejecutar el binario, instalar nada ni decidir compatibilidad.
  Tampoco convierte iconos: reporta el `.icns` declarado y el original queda
  donde está.

`Info.plist` y las cabeceras Mach-O son entrada no confiable: una clave del tipo
equivocado se rechaza en lugar de ignorarse, y un plist que no es diccionario o
no declara identificador ni ejecutable se rechaza nombrando el archivo.

---

## Application Library

**Crate:** `dp-app-library` · **Estado:** implementado (Fase 3, corte 1) para
catálogo e historial

Catálogo local en SQLite (`darling-platform/library.sqlite3` en el data home de
la plataforma), fuera del DPREFIX, con migraciones versionadas en
`PRAGMA user_version`.

- **Hace:** persiste las dos tablas del corte, `applications` (identidad,
  versión, rutas, arquitectura, tipo y estado de instalación) e `installations`
  (origen, formato, resultado, motivo y fecha de cada intento, con éxito o sin
  él). Escribe aplicación e intento en una sola transacción y ofrece un *upsert*
  para el reemplazo. Se consulta con `dp app list [--json]`.
- **No hace:** ser la única fuente de verdad sobre el sistema de archivos: antes
  de lanzar, actualizar o desinstalar hay que verificar que las rutas grabadas
  existen.

Estados distintos que no deben colapsarse: `Imported`, `Installed`, `Analyzed`,
`Compatible`, `PartiallyCompatible`, `Unsupported`, `Running`, `Crashed`. Una
aplicación puede estar instalada y ser no ejecutable a la vez.

**No hace todavía:** `running_instances` y el estado de ejecución (nada lanza
procesos en este corte), el análisis de compatibilidad en la fila y la consulta
de historial desde el CLI. La tabla prevista `installation_history` se llamó
`installations` al implementarse.

---

## Compatibility Engine

**Crate:** `dp-compatibility-engine` · **Estado:** implementado (Fase 4, corte 1)

Analiza estáticamente los requisitos de una aplicación macOS y los compara con un
perfil versionado de las capacidades del Darling fijado. No hay análisis
dinámico todavía.

- **Hace:** lee arquitecturas y slices del ejecutable, dependencias `dylib`,
  frameworks referenciados y la versión mínima de macOS declarada; compara cada
  requisito con `deps/darling-capabilities.toml`, que viaja embebido en el binario
  con `include_str!` y se valida en cada carga; y deriva el veredicto del peor
  requisito, con una evidencia por requisito. Se expone con
  `dp app compat <path> [--json]`, que nunca resuelve el prefijo, no escribe nada
  y no condiciona la instalación.
- **No hace:** ejecutar la aplicación para inspeccionarla, resolver install names
  (`@rpath` incluido), ni convertir un resultado favorable en garantía.

Salida: `schema_version`, `verdict`, `analyser_version`, `profile_version` y un
`requirements[]` con `kind`, `subject`, `status` y `detail`. Veredictos,
severidad, perfil y límites están en
[`application-compat.md`](application-compat.md).

**No hace todavía:** análisis de símbolos y APIs, detección de DRM/anticheat,
requisitos gráficos, de audio y de entrada, ni la matriz inicial de aplicaciones
de prueba. `dp app install` no consulta el motor.

---

## Runtime y perfiles

**Crate:** futuro · **Estado:** no existe

El núcleo de ejecución es compartido. Los perfiles aportan políticas y servicios,
no copias de Darling.

- **Ejecución por categoría:** políticas y selección de dispositivo y backend
  según el caso de uso. - Perfiles por categoría (productividad, desarrollo,
  multimedia). - Entrada, audio, latencia y diagnóstico según el contexto.
- **Diagnóstico por categoría:** suites de diagnóstico y validación según el
  caso de uso.

---

## CLI (`dp`)

**Crate:** `dp-cli` · **Estado:** `prefix`, `hardware scan` y `app` implementados

Implementa `--version`, `--help`, el rechazo explícito de entrada desconocida
(código de salida 2) y las familias `dp prefix`, `dp hardware` y `dp app`. Cada
comando es un adaptador delgado: la lógica vive en el módulo que el comando
invoca, y la salida JSON se deriva de los tipos del módulo, no de plantillas
paralelas.

`dp app inspect <path>` es de solo lectura y no toca el prefijo ni el catálogo;
`dp app install <path> [--replace]` toma el lock de escritura del Shared Prefix
Manager antes de escribir; `dp app list [--json]` abre el catálogo, que se crea
vacío si no existe; `dp app compat <path>` analiza el bundle sin resolver siquiera
la ubicación del prefijo. Todos aceptan `--json`. `inspect`, `install` y `list`
usan código 1 para un fallo de ejecución y 2 para un uso incorrecto; `compat` sale
con 0 cuando el veredicto es `compatible` o `partial` y con 1 cuando es
`not_compatible`, `unknown` o el análisis falló. Detalle en
[`application-install.md`](application-install.md) y
[`application-compat.md`](application-compat.md).

`dp hardware scan` acepta `--json` y sale con código 1 solo cuando el reporte
llega `failed`, para que un script distinga un reporte degradado pero usable de
uno inútil. Vulkan no está entre los comandos previstos hasta su propio corte
(ADR-0011).

---

## Application Center

**Crate:** futuro · **Estado:** no existe

Biblioteca visual tipo launcher: iconos, nombre, versión, estado de instalación y
compatibilidad, acciones (iniciar, detener, inspeccionar, configurar,
desinstalar), progreso, historial y diagnósticos. Consume el daemon y los
contratos públicos; nunca accede directamente al DPREFIX.
