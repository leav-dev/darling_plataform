# Componentes

Un apartado por componente: qué hace, qué no hace, qué contrato expone y en qué
estado está hoy. Los contratos son **conceptuales**: nombres y campos pueden
cambiar hasta que exista implementación y pruebas que los fijen.

Estado actual: todos los crates están vacíos y declaran su límite en la
documentación del crate. Ningún componente tiene lógica todavía.

---

## Hardware Intelligence Engine (DHI)

**Crate:** `dp-hardware-intelligence` · **Estado:** esqueleto

Produce un perfil verificable del host: arquitectura, modelo y features de CPU,
memoria, distribución, kernel, libc, GPUs por PCI/sysfs, capacidades Vulkan
reales (dispositivos, extensiones, propiedades y features consultadas) y sesión
gráfica cuando esté disponible.

- **Hace:** detecta, normaliza y publica un reporte versionado con diagnósticos
  por subsistema.
- **No hace:** instalar drivers, modificar el host, elegir GPU o backend.
- **Reglas:** sin root; tolerante a fallos; no infiere features Vulkan a partir de
  la versión de API; correlaciona PCI con Vulkan para no duplicar dispositivos;
  identificadores estables (por ejemplo PCI BDF); sin `-march=native`.

```rust
pub struct HardwareReport {
    pub schema_version: String,
    pub scan_status: ScanStatus,   // Complete | Partial | Failed
    pub system: SystemInfo,
    pub cpu: Option<CpuInfo>,
    pub gpus: Vec<GpuInfo>,
    pub memory: Option<MemoryInfo>,
    pub displays: Vec<DisplayInfo>,
    pub diagnostics: Vec<Diagnostic>,
}
```

El reporte se versiona para que el runtime evolucione sin romper consumidores.

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
| Consulta | `dg prefix status [--json]` | Reporta ubicación, fase, interrupción, lock ocupado y advertencias; no escribe nada |
| Validación | `dg prefix validate [--json]` | Diez checks: existencia, **directorio padre**, tipo, symlink, propiedad, escritura, longitud, ubicación dedicada, datos fuera del prefijo y registro de estado |
| Arranque | `dg prefix start` | Crea el **directorio padre** del prefijo, luego `darling shell /usr/bin/true` con `DPREFIX` exportado; persiste `Initializing`/`Starting` antes de invocar |
| Apagado | `dg prefix stop [--active-instances N]` | Se niega con instancias activas; si no, `darling shutdown` |

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

**Crate:** `dp-app-installer` · **Estado:** esqueleto

Importa `.app`, ZIP y —cuando el flujo esté verificado— DMG y PKG. Flujo:
seleccionar → identificar formato → extraer o montar en staging → inspeccionar
metadatos → validar estructura y arquitectura → analizar requisitos → decidir si
la instalación está permitida → instalar en el prefijo → registrar en el catálogo →
actualizar icono y catálogo.

- **No hace:** declarar compatibilidad, ejecutar scripts de instalador sin
  consentimiento, ni sobrescribir una instalación existente sin política explícita.
- **Riesgo:** los archivos son entrada no confiable. Staging obligatorio, rechazo
  de path traversal y de symlinks maliciosos, y PKG tratado como flujo de mayor
  riesgo con advertencia de que los scripts pueden impedir un rollback completo.

Prioridad de formatos: `.app` → `.zip` → `.dmg` → `.pkg` → `.xip`.

---

## Bundle Inspector

**Crate:** `dp-bundle-inspector` · **Estado:** esqueleto

Extrae metadatos **sin ejecutar** el binario: `CFBundleIdentifier`,
`CFBundleName`, `CFBundleDisplayName`, `CFBundleVersion`, `CFBundleExecutable`,
`CFBundleIconFile`, `LSMinimumSystemVersion`, arquitecturas y slices Mach-O, y
rutas de frameworks y recursos.

Valida que las rutas declaradas permanezcan dentro del bundle y que el ejecutable
esperado exista. `Info.plist` y las cabeceras Mach-O son entrada no confiable.

---

## Application Library

**Crate:** `dp-app-library` · **Estado:** esqueleto

Catálogo local en SQLite, fuera del DPREFIX, con migraciones versionadas. Tablas
iniciales previstas: `applications`, `running_instances`, `installation_history`.

- **Hace:** persiste identidad, versión, rutas, estado de instalación,
  historial y estado de ejecución.
- **No hace:** ser la única fuente de verdad sobre el sistema de archivos.

Estados distintos que no deben colapsarse: `Imported`, `Installed`, `Analyzed`,
`Compatible`, `PartiallyCompatible`, `Unsupported`, `Running`, `Crashed`. Una
aplicación puede estar instalada y ser no ejecutable a la vez.

---

## Compatibility Engine

**Crate:** `dp-compatibility-engine` · **Estado:** esqueleto

Analiza requisitos estáticamente y, cuando sea posible, dinámicamente, y los
compara con un perfil versionado de capacidades del Darling fijado y del host.

- **Hace:** Mach-O (arquitectura y slices), dependencias `dylib`, frameworks
  referenciados, versión mínima de macOS, metadatos del bundle y requisitos
  gráficos, de audio y de entrada identificables.
- **No hace:** ejecutar la aplicación para inspeccionarla, ni convertir un
  resultado favorable en garantía.

Salida: resultado general, evidencias, requisitos satisfechos y no satisfechos,
elementos desconocidos, advertencias y versiones del analizador y del perfil.

---

## Runtime y perfiles

**Crate:** futuro · **Estado:** no existe

El núcleo de ejecución es compartido. Los perfiles aportan políticas y servicios,
no copias de Darling.

- **Gaming Runtime:** política Dedicated GPU First, selección de dispositivo y
  backend, perfiles por juego, entrada, audio, latencia y suite de regresión.
- **Productividad / desarrollo / multimedia:** perfiles de entorno cuando hagan
  falta; no se crea un runtime separado sin una diferencia funcional real.

---

## CLI (`dg`)

**Crate:** `dp-cli` · **Estado:** `prefix` implementado

Implementa `--version`, `--help`, el rechazo explícito de entrada desconocida
(código de salida 2) y la familia `dg prefix`. Cada comando es un adaptador
delgado: la lógica vive en el módulo que el comando invoca, y la salida JSON se
deriva de los tipos del módulo, no de plantillas paralelas.

Próximo comando previsto: `dg hardware scan` (Fase 1 del plan).

---

## Application Center

**Crate:** futuro · **Estado:** no existe

Biblioteca visual tipo launcher: iconos, nombre, versión, estado de instalación y
compatibilidad, acciones (iniciar, detener, inspeccionar, configurar,
desinstalar), progreso, historial y diagnósticos. Consume el daemon y los
contratos públicos; nunca accede directamente al DPREFIX.
