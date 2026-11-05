# Plan de desarrollo y arquitectura --- Darling Platform

> **Estado del documento:** propuesta base de arquitectura y hoja de
> ruta\
> **Fecha:** 2026-11-03\
> **Audiencia:** colaboradores, mantenedores y futuros contribuidores\
> **Propósito:** conservar las decisiones técnicas y de producto,
> establecer reglas de desarrollo y ofrecer una dirección arquitectónica
> común.

------------------------------------------------------------------------

## 1. Visión del proyecto

El proyecto busca proporcionar en Linux una plataforma para **importar,
instalar, administrar y ejecutar aplicaciones diseñadas para macOS**,
utilizando Darling como núcleo de compatibilidad y construyendo
alrededor de él herramientas de gestión, diagnóstico, integración con el
escritorio y selección de capacidades.

La plataforma debe separar claramente dos objetivos:

1.  **Compatibilidad general:** aplicaciones de productividad,
    desarrollo, diseño, multimedia y otras categorías.
2.  **Compatibilidad por categorías:** perfiles,
    gráficos, entrada, audio, rendimiento, diagnóstico y
    pruebas por caso de uso.

El nombre **Darling Platform** se utiliza en este documento como nombre
de trabajo para la plataforma general. No representa una decisión
irreversible sobre el nombre o la organización definitiva de los
repositorios. **dpdarling plataform** puede mantenerse como nombre de una
edición o runtime especializado en videojuegos.

### 1.1 Objetivos

-   Reutilizar Darling y contribuir upstream cuando una mejora sea
    genérica.
-   Ofrecer un centro visual de aplicaciones instaladas e importadas.
-   Administrar un entorno Darling compartido por usuario.
-   Ejecutar varias aplicaciones de forma concurrente cuando Darling lo
    permita.
-   Analizar requisitos y limitaciones antes de intentar ejecutar una
    aplicación.
-   Detectar hardware y capacidades reales del sistema sin modificarlo.
-   Mantener una arquitectura modular, verificable y extensible.
-   Soportar progresivamente distintas distribuciones Linux y
    arquitecturas.
-   Diferenciar instalación, análisis, compatibilidad y ejecución: una
    aplicación instalada no necesariamente es ejecutable.

### 1.2 Fuera de alcance inicial

-   Reimplementar Darwin, Mach, dyld, Objective-C Runtime o servicios
    que ya proporciona Darling.
-   Crear un intérprete de binarios macOS independiente.
-   Garantizar compatibilidad universal con aplicaciones.
-   Instalar o reemplazar automáticamente drivers del host.
-   Crear un prefijo Darling independiente por aplicación.
-   Ejecutar scripts de instaladores sin consentimiento y sin advertir
    sus efectos.
-   Prometer compatibilidad con aplicaciones complejas ---por ejemplo,
    Xcode, herramientas profesionales o juegos con anticheat--- antes de
    validarlas experimentalmente.

------------------------------------------------------------------------

## 2. Principios y reglas no negociables

Estas reglas deben orientar las decisiones de diseño, las revisiones de
código y las contribuciones.

### 2.1 Reutilizar Darling

-   Darling es el núcleo de compatibilidad macOS.
-   La plataforma lo integra y administra; no debe duplicar sus
    subsistemas internos.
-   Antes de implementar una capacidad, investigar si Darling ya la
    ofrece o si existe una mejora upstream apropiada.
-   Mantener un fork solo cuando sea necesario para cambios propios que
    todavía no estén integrados upstream.
-   Las mejoras genéricas deben proponerse upstream siempre que sea
    viable.
-   No asumir comandos, interfaces internas ni comportamientos de
    Darling sin comprobarlos contra la versión objetivo.

### 2.2 Un único DPREFIX compartido

**Decisión confirmada:** todas las aplicaciones administradas por la
plataforma utilizarán un único DPREFIX dedicado.

-   No crear un DPREFIX por aplicación.
-   El prefijo será independiente del prefijo convencional del usuario,
    si existe.
-   La ubicación propuesta es `~/.local/share/darling-platform/prefix/`.
-   La plataforma debe configurar `DPREFIX` explícitamente al invocar
    Darling.
-   Darling debe administrar la estructura interna del prefijo. No se
    deben fabricar manualmente directorios internos que correspondan al
    sistema de Darling.
-   Las aplicaciones se instalarán en ubicaciones apropiadas dentro del
    entorno, por ejemplo `/Applications`, cuando el flujo soportado por
    Darling lo permita.
-   Los datos de catálogo, iconos, logs y configuración de la plataforma
    permanecerán fuera del DPREFIX.

Consecuencias que deben tratarse explícitamente: - Las aplicaciones
comparten bibliotecas, servicios y estado del entorno. - Una instalación
o actualización puede afectar a otras aplicaciones. - Las operaciones
que modifican el entorno necesitan exclusión mutua. - Las aplicaciones
deben poder ejecutarse concurrentemente si el runtime y Darling lo
soportan. - No se debe detener el entorno mientras existan procesos
gestionados activos. - El aislamiento por aplicación no forma parte del
MVP; cualquier incorporación futura requiere una decisión de
arquitectura documentada.

### 2.3 Un daemon de ejecución por sesión de usuario

-   Utilizar un servicio administrador persistente por sesión de
    usuario, activado bajo demanda.
-   Preferir D-Bus de sesión y `systemd --user` cuando estén
    disponibles.
-   No crear un daemon por aplicación.
-   No sustituir ni reimplementar `darlingserver`, `launchd` u otros
    componentes internos de Darling.
-   El daemon supervisa las aplicaciones y coordina el ciclo de vida del
    DPREFIX.
-   El frontend no debe enviar comandos de shell arbitrarios al daemon:
    debe utilizar operaciones tipadas y validadas.

### 2.4 Detección de hardware frente a decisiones de ejecución

-   El Hardware Intelligence Engine (DHI) descubre hardware y
    capacidades.
-   El DHI no decide por sí solo cómo ejecutar una aplicación.
-   El runtime utiliza el perfil del DHI y las políticas del usuario
    para seleccionar dispositivos y backends.
-   La detección debe ser parcial y tolerante a fallos: que Vulkan no
    esté disponible no debe impedir detectar CPU, memoria, distribución
    o dispositivos PCI.

### 2.5 Política de GPU: Dedicated GPU First

Cuando el sistema tenga GPU dedicada e integrada:

1.  Detectar y conservar ambas.
2.  Priorizar la GPU dedicada para renderizado y ejecución de
    aplicaciones.
3.  Mantener la GPU integrada disponible para pantallas, presentación,
    selección de dispositivos y sistemas híbridos.
4.  Evaluar la integrada como alternativa si la dedicada no está
    disponible o no cumple los requisitos.
5.  Permitir selección manual cuando sea técnicamente posible.

La integrada no debe descartarse ni competir como candidata de
renderizado predeterminada mientras exista una dedicada funcional y
adecuada. Esta política se aplica en el runtime; el DHI debe describir
los dispositivos sin ocultarlos.

### 2.6 Seguridad y respeto al sistema anfitrión

-   No requerir root para el escaneo normal de hardware ni para las
    operaciones ordinarias de usuario.
-   No instalar drivers ni modificar configuraciones del host
    automáticamente.
-   No ejecutar una aplicación para inspeccionar sus metadatos.
-   Tratar bundles, archivos comprimidos, DMG y PKG como entradas no
    confiables.
-   Validar rutas y evitar path traversal, enlaces simbólicos maliciosos
    y sobrescrituras fuera de los destinos permitidos.
-   Solicitar consentimiento antes de ejecutar instaladores que puedan
    lanzar scripts o realizar cambios.
-   Registrar diagnósticos sin exponer datos privados innecesarios.
-   No aceptar rutas o argumentos arbitrarios que permitan al frontend
    convertir el daemon en un ejecutor de comandos genérico.

### 2.7 Compatibilidad basada en evidencia

-   No inferir compatibilidad únicamente a partir de la arquitectura
    Mach-O o de `Info.plist`.
-   Registrar qué se detectó, qué se pudo comprobar y qué permanece
    desconocido.
-   No equiparar "instalada" con "compatible".
-   Toda afirmación de compatibilidad debe estar respaldada por pruebas
    reproducibles y una versión concreta del runtime.

------------------------------------------------------------------------

## 3. Arquitectura conceptual

La arquitectura se divide en un núcleo de plataforma, subsistemas de
administración, runtimes especializados e integración con el escritorio.

``` mermaid
flowchart TB
    UI["Darling Application Center<br/>Biblioteca y experiencia de usuario"]
    CLI["CLI / herramientas de administración"]
    IPC["API local tipada<br/>D-Bus de sesión"]
    DEM["Darling Execution Manager<br/>Daemon por sesión"]
    INSTALL["Darling App Installer"]
    LIB["Application Library<br/>Catálogo y metadatos"]
    COMP["Compatibility Engine"]
    SPM["Shared Prefix Manager"]
    DHI["Hardware Intelligence Engine"]
    RUNTIME["Runtime / políticas de ejecución"]
    DARLING["Darling Core<br/>Mach-O, dyld, Darwin APIs"]
    HOST["Linux Host<br/>Kernel, libc, drivers, Vulkan, audio, display"]
    PROFILE["Perfiles de ejecución<br/>productividad, desarrollo, multimedia"]
    OTHER["Otros perfiles de runtime<br/>productividad, desarrollo, multimedia"]

    UI --> IPC
    CLI --> IPC
    IPC --> DEM
    DEM --> LIB
    DEM --> SPM
    DEM --> COMP
    DEM --> RUNTIME
    INSTALL --> LIB
    INSTALL --> SPM
    COMP --> DARLING
    DHI --> RUNTIME
    HOST --> DHI
    RUNTIME --> DARLING
    PROFILE --> RUNTIME
    OTHER --> RUNTIME
    SPM --> DARLING
    DARLING --> HOST
```

### 3.1 Capas y responsabilidades

  -----------------------------------------------------------------------
  Capa                    Responsabilidad         No debe hacer
  ----------------------- ----------------------- -----------------------
  Application Center      Mostrar aplicaciones,   Ejecutar directamente
                          estados, acciones y     binarios o administrar
                          diagnósticos            el DPREFIX

  CLI                     Administración,         Duplicar lógica de
                          diagnóstico y           negocio
                          automatización          

  IPC/API                 Contratos entre         Exponer ejecución
                          frontend, CLI y daemon  arbitraria de shell

  Execution Manager       Coordinar lanzamientos, Reimplementar servicios
                          procesos y ciclo de     internos de Darling
                          vida                    

  App Installer           Importar, inspeccionar  Declarar compatibilidad
                          y preparar              sin análisis
                          instalaciones           

  Application Library     Persistir metadatos y   Ser la única fuente de
                          estados                 verdad sobre archivos
                                                  físicos

  Compatibility Engine    Analizar requisitos y   Garantizar que una
                          limitaciones            aplicación funcionará
                                                  solo por análisis
                                                  estático

  Shared Prefix Manager   Inicializar, validar,   Mantener un prefijo por
                          bloquear y detener el   aplicación
                          DPREFIX                 

  Hardware Intelligence   Detectar hardware y     Instalar drivers o
  Engine                  capacidades             tomar decisiones de
                                                  lanzamiento

  Runtime                 Seleccionar estrategia  Ocultar la evidencia o
                          de ejecución y          inventar capacidades
                          dispositivos            

  Darling Core            Proveer compatibilidad  Ser duplicado por la
                          macOS                   plataforma
  -----------------------------------------------------------------------

------------------------------------------------------------------------

## 4. Estructura de repositorio propuesta

Se recomienda comenzar con un **monorepo**. Esto facilita mantener
contratos coherentes, ejecutar pruebas integradas y versionar
conjuntamente los módulos mientras la arquitectura evoluciona.

``` text
darling-platform/
├── README.md
├── CONTRIBUTING.md
├── CODE_OF_CONDUCT.md
├── LICENSE
├── SECURITY.md
├── DEVELOPMENT.md
├── PLAN.md
├── CHANGELOG.md
├── CMakeLists.txt
├── Cargo.toml
├── Cargo.lock
│
├── cmake/
│   ├── Rust.cmake
│   ├── DarlingExternal.cmake
│   └── Packaging.cmake
│
├── docs/
│   ├── architecture/
│   │   ├── overview.md
│   │   ├── components.md
│   │   ├── data-flow.md
│   │   └── decisions/
│   │       ├── ADR-0001-monorepo.md
│   │       ├── ADR-0002-shared-dprefix.md
│   │       ├── ADR-0003-execution-daemon.md
│   │       └── ADR-0004-gpu-selection-policy.md
│   ├── development/
│   ├── compatibility/
│   └── packaging/
│
├── apps/
│   ├── application-center/
│   └── cli/
│
├── services/
│   └── execution-manager/
│
├── modules/
│   ├── application-management/
│   │   ├── app-installer/
│   │   ├── app-library/
│   │   └── bundle-inspector/
│   ├── compatibility-engine/
│   ├── shared-prefix/
│   ├── hardware-intelligence/
│   ├── runtime/
│   └── platform-common/
│
├── runtimes/
│   ├── general/
│   ├── productivity/
│   ├── development/
│   └── multimedia/
│
├── integrations/
│   ├── darling/
│   ├── dbus/
│   ├── desktop/
│   ├── graphics/
│   │   ├── vulkan/
│   │   ├── metal/
│   │   └── opengl/
│   ├── audio/
│   └── input/
│
├── packaging/
│   ├── deb/
│   ├── rpm/
│   ├── arch/
│   ├── nix/
│   └── flatpak/
│
├── tests/
│   ├── unit/
│   ├── integration/
│   ├── compatibility/
│   ├── fixtures/
│   └── test-apps/
│
└── scripts/
    ├── dev/
    ├── ci/
    └── release/
```

La estructura es una dirección objetivo, no una obligación de crear
todos los directorios vacíos desde el primer commit. Los módulos deben
incorporarse cuando exista una primera implementación útil y
verificable.

### 4.1 Organización de código Rust

Los módulos Rust deben ser crates separados cuando tengan límites
claros, contratos estables o necesidad de reutilización independiente.
Evitar crear crates por cada archivo o abstracción pequeña.

Workspace inicial orientativo:

``` toml
[workspace]
resolver = "2"
members = [
    "modules/platform-common",
    "modules/hardware-intelligence",
    "modules/shared-prefix",
    "modules/compatibility-engine",
    "modules/application-management/app-installer",
    "modules/application-management/app-library",
    "modules/application-management/bundle-inspector",
    "services/execution-manager",
]
```

Las rutas definitivas y los nombres de crates deben ajustarse al
implementar el primer vertical funcional.

------------------------------------------------------------------------

## 5. Componentes principales

### 5.1 Hardware Intelligence Engine (DHI)

**Objetivo:** producir un perfil verificable del hardware y del entorno
gráfico del host.

Responsabilidades: - Arquitectura, modelo, topología y características
de CPU. - Memoria del sistema. - Distribución, kernel y libc cuando
pueda determinarse. - Enumeración de GPUs, incluyendo dispositivos
PCI. - Información Vulkan real: dispositivos, extensiones, propiedades y
features consultadas. - Información básica de sesión gráfica y pantallas
cuando esté disponible. - Diagnósticos por subsistema y estado de
descubrimiento.

Reglas: - Implementación principal en Rust. - No requerir privilegios
elevados para el escaneo habitual. - No fallar globalmente por un
proveedor individual. - No inferir features Vulkan a partir únicamente
de la versión de API. - Enumerar GPUs por PCI/sysfs y correlacionarlas
con Vulkan para reducir duplicados. - Mantener identificadores estables
cuando sea posible, como PCI BDF. - Preparar proveedores específicos
para x86-64 y AArch64. - No compilar con `-march=native` para distribuir
binarios genéricos.

Modelo conceptual simplificado:

``` rust
pub struct HardwareReport {
    pub schema_version: String,
    pub scan_status: ScanStatus,
    pub system: SystemInfo,
    pub cpu: Option<CpuInfo>,
    pub gpus: Vec<GpuInfo>,
    pub memory: Option<MemoryInfo>,
    pub displays: Vec<DisplayInfo>,
    pub diagnostics: Vec<Diagnostic>,
}

pub enum ScanStatus {
    Complete,
    Partial,
    Failed,
}

pub enum SupportLevel {
    Supported,
    Partial,
    Unsupported,
    Unknown,
}
```

Proveedores y fuentes candidatas: - x86-64: `raw-cpuid`. - AArch64:
`getauxval`, HWCAP/HWCAP2 y fuentes complementarias. - Linux:
`/etc/os-release`, `/proc`, `/sys`. - GPU: PCI sysfs y Vulkan Loader
mediante una biblioteca como `ash`.

El reporte JSON debe estar versionado para que el runtime pueda
evolucionar sin romper consumidores.

### 5.2 Shared Prefix Manager (SPM)

**Objetivo:** ser la única autoridad de la plataforma sobre el ciclo de
vida del DPREFIX compartido.

Responsabilidades: - Inicializar el prefijo mediante los mecanismos
soportados por Darling. - Validar su estructura y disponibilidad. -
Iniciar y detener el entorno. - Mantener locks y coordinar
operaciones. - Evitar instalaciones, actualizaciones o desinstalaciones
concurrentes incompatibles. - Recuperarse de interrupciones y
operaciones incompletas. - Impedir el apagado si existen procesos
gestionados activos.

Estados conceptuales:

``` rust
pub enum PrefixState {
    NotInitialized,
    Initializing,
    Stopped,
    Starting,
    Running,
    Busy,
    Stopping,
    Error,
}
```

Estos estados pertenecen al orquestador de la plataforma; no deben
presentarse como estados internos oficiales de Darling si no lo son.

Política de concurrencia: - Varias aplicaciones pueden ejecutarse
simultáneamente si Darling lo permite. - Operaciones de lectura y
consulta pueden coexistir con aplicaciones en ejecución. - Instalación,
actualización y desinstalación deben obtener un lock de escritura. -
Operaciones que alteren componentes compartidos pueden requerir detener
aplicaciones. - El apagado del prefijo requiere confirmar que no quedan
procesos activos.

### 5.3 Darling Execution Manager (DEM)

**Objetivo:** administrar lanzamientos y procesos desde un daemon por
sesión de usuario.

Responsabilidades: 1. Recibir una solicitud tipada de lanzamiento. 2.
Resolver el identificador de la aplicación en el catálogo. 3. Verificar
que el bundle y el ejecutable existen. 4. Consultar el análisis de
compatibilidad y advertir sobre riesgos conocidos. 5. Comprobar/iniciar
el DPREFIX. 6. Preparar el entorno de ejecución. 7. Lanzar la aplicación
utilizando mecanismos confirmados para la versión de Darling integrada.
8. Supervisar el proceso y registrar su estado. 9. Notificar eventos al
Application Center. 10. Mantener el DPREFIX activo mientras haya
aplicaciones gestionadas.

Contrato conceptual:

``` rust
pub struct LaunchRequest {
    pub app_id: String,
    pub arguments: Vec<String>,
}

pub struct RunningApplication {
    pub instance_id: String,
    pub app_id: String,
    pub host_pid: Option<u32>,
    pub started_at: String,
    pub status: ApplicationStatus,
}

pub enum ApplicationStatus {
    Starting,
    Running,
    Stopping,
    Stopped,
    Crashed,
    Unknown,
}
```

`host_pid` representa el PID supervisado desde Linux. No debe
confundirse con un PID de proceso dentro del entorno Darwin.

Interfaz local propuesta: - Bus de sesión D-Bus. - Activación bajo
demanda y unidad `systemd --user` cuando esté disponible. - Métodos
tipados como `LaunchApp`, `StopApp`, `GetAppStatus`, `ListRunningApps`,
`GetPrefixStatus`. - Eventos como `AppStarted`, `AppStopped`,
`AppCrashed` y `PrefixStateChanged`.

Los nombres de servicio, objetos y métodos D-Bus son propuestas de
diseño y deben validarse antes de convertirse en API estable.

### 5.4 Darling App Installer (DAI)

**Objetivo:** importar aplicaciones, inspeccionar sus bundles y realizar
instalaciones controladas.

Flujo esperado:

``` mermaid
flowchart TD
    A["Seleccionar archivo o bundle"] --> B["Identificar formato"]
    B --> C["Extraer o montar en staging"]
    C --> D["Inspeccionar metadatos"]
    D --> E["Validar estructura y arquitectura"]
    E --> F["Analizar requisitos"]
    F --> G{"¿Instalación permitida?"}
    G -- "No" --> H["Mostrar motivo y cancelar"]
    G -- "Sí" --> I["Instalar en DPREFIX compartido"]
    I --> J["Registrar en Application Library"]
    J --> K["Actualizar icono y catálogo"]
```

Formatos por prioridad: 1. `.app` (bundle ya extraído). 2. `.zip`. 3.
`.dmg`. 4. `.pkg`, con advertencias y consentimiento explícito. 5.
`.xip`, después de investigar soporte y herramientas adecuadas.

No se debe asumir que todos los formatos o variantes son compatibles con
todas las versiones de Darling.

Reglas: - Extraer primero en staging. - Validar rutas antes de copiar
archivos. - No sobrescribir una instalación existente sin política
explícita. - Registrar el origen, versión y resultado de la
instalación. - Para bundles copiados o ZIP, diseñar una estrategia de
rollback. - Para PKG, advertir que scripts y efectos externos pueden
impedir un rollback completo. - No ejecutar scripts de instalación de
forma silenciosa. - Convertir iconos `.icns` a un formato adecuado para
la interfaz, conservando el original cuando sea útil.

### 5.5 Bundle Inspector

Debe extraer metadatos sin ejecutar el binario:

-   `CFBundleIdentifier`
-   `CFBundleName`
-   `CFBundleDisplayName`
-   `CFBundleVersion`
-   `CFBundleExecutable`
-   `CFBundleIconFile`
-   `LSMinimumSystemVersion`
-   Arquitecturas y slices Mach-O.
-   Rutas de frameworks y recursos del bundle.

Debe validar que los paths declarados permanezcan dentro del bundle y
que el ejecutable esperado exista.

### 5.6 Application Library

**Objetivo:** mantener el catálogo local de aplicaciones, instalaciones
e instancias.

Almacenamiento inicial: - SQLite. - Datos fuera del DPREFIX. - Iconos y
cachés separados. - Migraciones de esquema versionadas.

Tablas iniciales orientativas:

``` sql
CREATE TABLE applications (
    id TEXT PRIMARY KEY,
    bundle_id TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT,
    bundle_path TEXT NOT NULL,
    executable_path TEXT NOT NULL,
    icon_path TEXT,
    architecture TEXT,
    installation_type TEXT NOT NULL,
    installed_at TEXT NOT NULL,
    installation_status TEXT NOT NULL
);

CREATE TABLE running_instances (
    instance_id TEXT PRIMARY KEY,
    app_id TEXT NOT NULL,
    host_pid INTEGER,
    started_at TEXT NOT NULL,
    status TEXT NOT NULL,
    FOREIGN KEY (app_id) REFERENCES applications(id)
);

CREATE TABLE installation_history (
    id TEXT PRIMARY KEY,
    app_id TEXT,
    source_path TEXT,
    format TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    result TEXT NOT NULL,
    diagnostic TEXT
);
```

La base de datos no es la fuente única de verdad sobre los archivos.
Antes de lanzar, actualizar o desinstalar, el sistema debe verificar que
los paths y bundles siguen existiendo y son válidos.

Estados de aplicación que deben distinguirse: - `Imported` -
`Installed` - `Analyzed` - `Compatible` - `PartiallyCompatible` -
`Unsupported` - `Running` - `Crashed`

Una aplicación puede estar instalada y aparecer en la biblioteca aunque
el análisis determine que no puede ejecutarse.

### 5.7 Compatibility Engine

**Objetivo:** analizar estática y dinámicamente, cuando sea posible, los
requisitos de una aplicación y compararlos con las capacidades conocidas
de Darling y del host.

Análisis estático inicial: - Arquitectura Mach-O y slices. - Tipo de
binario y ejecutable principal. - Dependencias dinámicas (`dylib`). -
Frameworks referenciados. - Versión mínima de macOS declarada. -
Metadatos del bundle. - Recursos y componentes requeridos
identificables.

Análisis ampliado: - APIs y símbolos importados cuando puedan
inspeccionarse. - Requisitos gráficos, audio y entrada. - Dependencias
externas. - Restricciones conocidas de DRM, anticheat o servicios
externos. - Requisitos que dependen de una versión concreta de Darling.

El motor debe producir: - Resultado general. - Evidencias encontradas. -
Requisitos satisfechos. - Requisitos no satisfechos. - Elementos
desconocidos. - Advertencias y recomendaciones. - Versión del analizador
y del perfil de capacidades utilizado.

No convertir un análisis estático favorable en una garantía de
ejecución. La compatibilidad real requiere pruebas.

### 5.8 Runtime y perfiles especializados

El núcleo de ejecución debe ser compartido. Los perfiles especializados
aportan políticas y servicios adicionales, no copias independientes de
Darling.

**Ejecución por categoría** - Política y selección de dispositivo y backend
según el caso de uso. - Perfiles por categoría (productividad, desarrollo,
multimedia). - Entrada, audio, latencia y diagnóstico según el contexto.

No crear un runtime separado hasta que exista una diferencia funcional
real que justifique su mantenimiento.

### 5.9 Application Center

Interfaz visual tipo biblioteca/launcher: - Icono original o
convertido. - Nombre, versión y editor si puede obtenerse. - Estado de
instalación y compatibilidad. - Acciones: iniciar, detener,
inspeccionar, configurar y desinstalar. - Progreso de importación e
instalación. - Historial y diagnóstico de fallos. - Filtros por
categoría y estado.

La interfaz debe consumir el servicio y los contratos públicos; no debe
acceder directamente a estructuras internas del DPREFIX.

------------------------------------------------------------------------

## 6. Integración con Darling y el sistema Linux

### 6.1 Darling como dependencia externa

-   Mantener el build de Darling basado en CMake.
-   Mantener los módulos de la plataforma en Cargo cuando estén escritos
    en Rust.
-   Integrar Darling como dependencia externa o proceso de build
    separado, no como miembro del workspace Cargo.
-   Fijar y documentar la versión o commit de Darling utilizado para
    cada release.
-   Registrar parches propios y su motivo.
-   Evitar depender de interfaces internas no estables sin una capa
    adaptadora.

### 6.2 CMake y Cargo

CMake debe coordinar el proyecto completo y las dependencias nativas.
Cargo compila y prueba los crates Rust.

Requisitos de build: - Debug y Release. - Dependencias explícitas de
`Cargo.toml`, `Cargo.lock` y fuentes. - Directorios de salida separados
por configuración y arquitectura. - Instalación de binarios, recursos y
archivos D-Bus. - No hardcodear rutas de salida que fallen en
generadores multi-config. - CI debe ejecutar `cargo fmt --check`,
`cargo clippy -- -D warnings`, `cargo test` y build de CMake.

La integración inicial puede usar `add_custom_target`, pero debe
evolucionar hacia dependencias y reglas de instalación reproducibles.

### 6.3 Abstracción del host

La plataforma debe reutilizar: - Kernel y drivers Linux. - Vulkan Loader
y drivers gráficos del host. - Servicios de display y audio
disponibles. - Herramientas de sistema solo cuando se compruebe su
presencia y comportamiento.

No debe asumir que todos los hosts tienen systemd, Wayland, X11, Vulkan
o la misma libc.

------------------------------------------------------------------------

## 7. Soporte de plataformas

### 7.1 Distribuciones Linux

La detección debe basarse en información del sistema, especialmente
`/etc/os-release`, kernel, libc y capacidades disponibles. No debe
depender de que exista un gestor de paquetes concreto.

Objetivos progresivos: - Debian / Ubuntu y derivadas. - Arch Linux y
derivadas. - Fedora y derivadas RPM. - openSUSE. - NixOS. - Otras
distribuciones, según viabilidad.

La capa de empaquetado puede generar DEB, RPM, paquetes Arch y
expresiones Nix. El formato de paquete no debe mezclarse con la lógica
del runtime.

Flatpak puede evaluarse como canal de distribución, pero no debe
asumirse que por sí solo resuelve los requisitos de kernel, namespaces,
montaje y ejecución de Darling.

### 7.2 Arquitecturas

Prioridad inicial: 1. Linux x86-64. 2. Preparación de interfaces y CI
para AArch64. 3. Soporte funcional AArch64 cuando Darling y sus
dependencias lo permitan.

No asumir que el soporte ARM64 está completo por el simple hecho de que
Rust compile para esa arquitectura. La compatibilidad de Darling,
dependencias, binarios macOS y traducción de instrucciones debe
validarse por separado.

------------------------------------------------------------------------

## 8. Datos y directorios del usuario

Ubicación propuesta:

``` text
~/.local/share/darling-platform/
├── prefix/                     # DPREFIX compartido administrado por Darling
├── apps/
│   ├── registry.sqlite
│   ├── icons/
│   ├── cache/
│   └── staging/
├── logs/
│   ├── execution/
│   ├── installation/
│   └── daemon/
└── config/
    ├── runtime.json
    └── applications.json
```

Configuración de usuario y preferencias pueden residir en
`~/.config/darling-platform/`, siguiendo Xdp Base Directory
Specification. Los datos variables, cachés y logs deben separarse de la
configuración cuando corresponda.

No se debe asumir que el DPREFIX es seguro para moverlo o copiarlo
mientras Darling está activo. La plataforma debe documentar backup y
recuperación.

------------------------------------------------------------------------

## 9. Contratos, errores y observabilidad

### 9.1 Contratos

-   Definir tipos de dominio sin dependencias de interfaz gráfica.
-   Mantener IPC versionado.
-   Usar identificadores estables de aplicación e instancia.
-   No exponer detalles de implementación de Darling como API pública si
    pueden encapsularse.
-   Documentar cambios incompatibles.

### 9.2 Errores

Los errores deben ser tipados y contener: - Subsistema que falló. -
Operación solicitada. - Código o categoría estable. - Mensaje legible. -
Diagnóstico técnico opcional. - Posibilidad de recuperación, si se
conoce.

Evitar errores genéricos que obliguen al frontend a interpretar cadenas
de texto.

### 9.3 Logs y diagnóstico

-   Logs estructurados.
-   Niveles configurables.
-   Correlación por `app_id`, `instance_id` y operación.
-   No incluir secretos ni contenido privado de aplicaciones.
-   Separar logs de daemon, instalación y ejecución.
-   Facilitar exportar un informe de diagnóstico con consentimiento del
    usuario.

------------------------------------------------------------------------

## 10. Plan de desarrollo por fases

La secuencia prioriza fundamentos verificables antes de construir una
interfaz completa.

### Fase 0 --- Investigación y validación técnica

**Objetivo:** eliminar supuestos riesgosos antes de comprometer
interfaces.

Tareas: - Auditar el estado actual de Darling y su documentación. -
Identificar cómo configurar y operar un DPREFIX. - Verificar los flujos
reales de ejecución de aplicaciones gráficas. - Probar instalación de
bundles `.app`, ZIP, DMG y PKG según soporte real. - Identificar
interfaces estables y no estables. - Revisar dependencias de build y
requisitos de kernel. - Crear una matriz inicial de capacidades
conocidas.

Entregables: - Informe de viabilidad. - Versión/commit de Darling de
referencia. - Lista de comandos y APIs verificadas. - Riesgos técnicos y
decisiones pendientes.

**Criterio de salida:** no diseñar el DEM o el instalador alrededor de
comandos de Darling que no hayan sido probados.

### Fase 1 --- Fundamentos del repositorio y DHI

**Objetivo:** crear la base de compilación y obtener un inventario de
hardware útil.

Tareas: - Crear monorepo, workspace Cargo y build CMake. - Configurar CI
de Rust y CMake. - Implementar modelos de dominio del DHI. - Detectar
distribución, kernel, libc y arquitectura. - Detectar CPU y
características soportadas. - Detectar memoria. - Enumerar GPUs PCI. -
Consultar Vulkan opcionalmente. - Correlacionar GPUs PCI y Vulkan. -
Generar reporte JSON versionado. - Implementar diagnósticos parciales.

Entregables: - CLI `dp-hardware scan`. - Salida JSON y texto. - Pruebas
unitarias y fixtures. - Documentación de campos y limitaciones.

**Criterio de salida:** el escaneo funciona sin root y la ausencia de
Vulkan no invalida el resto del reporte.

### Fase 2 --- Integración mínima de Darling y Shared Prefix Manager

**Objetivo:** demostrar que la plataforma puede administrar un DPREFIX
dedicado.

Tareas: - Integrar una versión fijada de Darling. - Definir la ubicación
del DPREFIX dedicado. - Implementar inicialización, validación, inicio y
apagado. - Implementar locks de lectura/escritura. - Comprobar procesos
activos antes de apagar. - Manejar estados interrumpidos y errores. -
Documentar recuperación y límites del filesystem.

Entregables: - Crate `shared-prefix`. - Pruebas de estado y
concurrencia. - Herramienta CLI para consultar y validar el prefijo.

**Criterio de salida:** el prefijo se inicializa y valida de forma
repetible sin modificar el prefijo Darling convencional del usuario.

### Fase 3 --- Bundle Inspector y App Installer

**Objetivo:** importar y registrar una aplicación sin necesidad de
lanzarla.

Tareas: - Inspeccionar `Info.plist`. - Detectar ejecutable, icono y
metadatos. - Analizar arquitectura Mach-O. - Implementar staging
seguro. - Incorporar primero bundles `.app` y ZIP. - Añadir DMG después
de verificar el flujo de Darling. - Tratar PKG como flujo explícito y de
mayor riesgo. - Implementar SQLite y migraciones. - Registrar historial
de instalación. - Añadir rollback donde sea posible.

Entregables: - Inspector CLI. - Instalador de primer formato
soportado. - Catálogo SQLite. - Tests con bundles de ejemplo y archivos
malformados.

**Criterio de salida:** una aplicación se importa, inspecciona, instala
y registra de forma repetible; los fallos no dejan instalaciones
parciales silenciosas.

### Fase 4 --- Compatibility Engine inicial

**Objetivo:** analizar requisitos antes de ejecutar.

Tareas: - Inspeccionar slices y arquitectura Mach-O. - Leer dependencias
dinámicas. - Identificar frameworks requeridos. - Leer versión mínima
declarada de macOS. - Comparar requisitos con un perfil versionado de
capacidades de Darling. - Reportar desconocidos y limitaciones. -
Diseñar el formato de evidencia y diagnóstico.

Entregables: - CLI de análisis. - Reporte de compatibilidad
versionado. - Matriz inicial de aplicaciones de prueba.

**Criterio de salida:** el sistema diferencia claramente compatible,
parcialmente compatible, no compatible y desconocido, con razones y
evidencias.

### Fase 5 --- Execution Manager y ejecución supervisada

**Objetivo:** lanzar y supervisar aplicaciones mediante un servicio por
sesión.

Tareas: - Diseñar protocolo IPC versionado. - Implementar activación
bajo demanda. - Conectar DEM con Application Library y SPM. -
Implementar lanzamiento con una interfaz Darling previamente
verificada. - Supervisar procesos y estados. - Manejar cierre normal,
terminación y crash. - Evitar apagar el DPREFIX con aplicaciones
activas. - Implementar logs y eventos.

Entregables: - Daemon de usuario. - Cliente CLI. - API D-Bus
documentada. - Pruebas de ciclo de vida y concurrencia.

**Criterio de salida:** el usuario puede iniciar, consultar y cerrar una
aplicación desde el cliente, y el daemon mantiene el estado coherente
ante fallos.

### Fase 6 --- Application Center

**Objetivo:** construir la biblioteca visual sobre APIs ya estables.

Tareas: - Vista de biblioteca. - Iconos y metadatos. - Estados de
instalación y compatibilidad. - Iniciar y detener. - Progreso de
instalación. - Detalle de aplicación y diagnósticos. - Desinstalación
controlada. - Integración opcional con menú de aplicaciones del
escritorio.

Entregables: - Primera interfaz funcional. - Integración completa con
DEM y DAI. - Documentación de experiencia de usuario.

**Criterio de salida:** todas las operaciones visibles se realizan
mediante servicios y contratos públicos, sin acceso directo de la
interfaz al DPREFIX.

### Fase 7 --- Optimización por categorías

**Objetivo:** añadir las capacidades específicas para videojuegos sin
afectar el núcleo general.

Tareas: - Aplicar políticas por categoría. - Seleccionar dispositivo y
backend según capacidades verificadas. - Investigar y probar traducciones
de API existentes. - Evaluar extensiones y features reales. - Incorporar
entrada, audio y diagnóstico según el caso de uso. - Construir suite de
diagnóstico por categoría. - Medir rendimiento y estabilidad. -
Documentar limitaciones de DRM y anticocheat.

Entregables: - Runtime inicial. - Perfiles versionados. - Matriz de
pruebas por categoría. - Diagnósticos por categoría.

**Criterio de salida:** cada caso de prueba listado tiene resultados
reproducibles, versión del entorno y hardware de prueba documentados.

### Fase 8 --- Distribución y ampliación de plataformas

**Objetivo:** empaquetar y ampliar el soporte con una base estable.

Tareas: - Paquetes DEB y RPM. - Paquete Arch. - Evaluación Nix. -
Integración `systemd --user` opcional. - Pruebas en varias
distribuciones. - Preparación y validación AArch64. - Firma,
actualización y publicación de releases. - Política de soporte y ciclo
de versiones.

**Criterio de salida:** los paquetes no dependen de rutas locales de
desarrollo y las instalaciones/actualizaciones se pueden reproducir en
entornos limpios.

------------------------------------------------------------------------

## 11. Estrategia de pruebas

### 11.1 Tipos de pruebas

-   **Unitarias:** parsing, modelos, validaciones, políticas y estados.
-   **Integración:** DHI, SQLite, SPM, IPC y Darling.
-   **Seguridad:** ZIP path traversal, symlinks, nombres malformados,
    permisos y rutas.
-   **Concurrencia:** múltiples lanzamientos, instalación durante
    ejecución y shutdown.
-   **Compatibilidad:** aplicaciones representativas por categoría.
-   **Regresión:** repetir casos que hayan fallado en versiones
    anteriores.
-   **Empaquetado:** instalación y actualización en distribuciones
    limpias.

### 11.2 Matriz de hardware

Incluir fixtures y pruebas reales, cuando se disponga de equipos,
para: - CPU x86-64 y AArch64. - GPU AMD, NVIDIA e Intel. - Sistemas con
GPU integrada únicamente. - Sistemas con GPU dedicada e integrada. -
Vulkan funcional y ausente. - Sesiones Wayland y X11 cuando
corresponda. - Diferentes versiones de kernel y libc.

Los fixtures permiten probar el análisis sin depender de tener todos los
equipos físicos, pero no sustituyen pruebas reales de drivers y
rendimiento.

### 11.3 Matriz de aplicaciones

Crear un conjunto pequeño y reproducible de aplicaciones de
referencia: - Aplicaciones sencillas de interfaz. - Herramientas de
productividad. - Aplicaciones basadas en frameworks conocidos. -
Aplicaciones con dependencias dinámicas relevantes. - Juegos 2D y 3D. -
Casos que se sabe que requieren APIs no soportadas.

Registrar por cada prueba: - Aplicación y versión. - Arquitectura. -
Versión de Darling y de la plataforma. - Distribución, kernel y libc. -
CPU y GPU/driver. - Resultado de análisis. - Resultado de instalación. -
Resultado de ejecución. - Logs y limitaciones observadas.

------------------------------------------------------------------------

## 12. Calidad, CI y convenciones

### 12.1 CI mínimo

En cada pull request: - `cargo fmt --check` -
`cargo clippy --workspace --all-targets -- -D warnings` -
`cargo test --workspace` - Build CMake en configuración Debug. - Build
Release en jobs programados o de release. - Análisis estático y
auditoría de dependencias según herramientas adoptadas. - Pruebas de
empaquetado en imágenes/VMs representativas.

### 12.2 Reglas de código

-   Preferir interfaces pequeñas y responsabilidades explícitas.
-   Separar dominio, aplicación e infraestructura.
-   No acoplar lógica de negocio a D-Bus, SQLite o UI.
-   Evitar estado global mutable.
-   Manejar errores explícitamente.
-   Documentar decisiones no obvias.
-   Añadir pruebas al corregir un bug.
-   No introducir dependencias sin justificar mantenimiento, licencia y
    necesidad.
-   No agregar una abstracción que todavía no tenga un consumidor real
    sin una razón clara.

### 12.3 Pull requests

Cada PR debe incluir: - Problema que resuelve. - Alcance y componentes
afectados. - Decisiones técnicas relevantes. - Pruebas ejecutadas. -
Cambios de esquema/API, si existen. - Riesgos y limitaciones
conocidas. - Actualización de documentación cuando cambie un
comportamiento o una decisión.

------------------------------------------------------------------------

## 13. Gestión de decisiones arquitectónicas

Este documento conserva las decisiones vigentes, pero no debe ser el
único mecanismo de control de cambios.

Usar **Architecture Decision Records (ADR)** en
`docs/architecture/decisions/`.

Cada ADR debe incluir: 1. Número y título. 2. Estado: propuesta,
aceptada, reemplazada o rechazada. 3. Contexto. 4. Decisión. 5.
Alternativas consideradas. 6. Consecuencias positivas y negativas. 7.
Fecha y responsables.

Decisiones iniciales que deben registrarse: - ADR-0001: monorepo como
estrategia inicial. - ADR-0002: un DPREFIX compartido y dedicado. -
ADR-0003: daemon de ejecución por sesión de usuario. - ADR-0004:
política Dedicated GPU First. - ADR-0005: DHI separado de las decisiones
de ejecución. - ADR-0006: Darling como núcleo externo de
compatibilidad. - ADR-0007: Rust para módulos de plataforma y CMake para
integración/build nativo.

Cuando una decisión cambie: - No borrar la decisión anterior. - Marcar
el ADR antiguo como reemplazado. - Crear un ADR nuevo que explique el
cambio. - Actualizar este plan y la documentación relacionada.

### 13.1 Registro resumido de decisiones actuales

  -----------------------------------------------------------------------
  Decisión                Estado                  Regla
  ----------------------- ----------------------- -----------------------
  Plataforma general para Dirección acordada      No limitar el núcleo a
  aplicaciones macOS                              videojuegos

  Darling como núcleo     Acordada                Reutilizar y contribuir
                                                  upstream

  Nombre Darling Platform Propuesto               Nombre de trabajo,
                                                  pendiente de
                                                  confirmación definitiva

  dpdarling plataform          Propuesto como          Runtime/edición general
                          especialización         sobre el núcleo
                                                  compartido

  DPREFIX compartido      Acordada                Un prefijo dedicado por
                                                  usuario

  DEM por sesión          Dirección acordada      Un daemon, múltiples
                                                  aplicaciones

  DHI en Rust             Acordada                Detección independiente
                                                  y tolerante a fallos

  CMake + Cargo           Dirección acordada      CMake coordina; Cargo
                                                  compila crates Rust

  Dedicated GPU First     Acordada                La dedicada es
                                                  prioritaria; la
                                                  integrada se conserva

  Aislamiento por         Fuera del MVP           Requiere ADR futuro
  aplicación                                      

  x86-64 primero          Prioridad acordada      AArch64 se prepara y
                                                  valida progresivamente
  -----------------------------------------------------------------------

------------------------------------------------------------------------

## 14. Riesgos técnicos principales

  -----------------------------------------------------------------------
  Riesgo                  Impacto                 Mitigación
  ----------------------- ----------------------- -----------------------
  Interfaces internas de  Alto                    Adaptadores, versiones
  Darling cambian                                 fijadas y pruebas de
                                                  integración

  Compatibilidad macOS    Alto                    Análisis con evidencias
  incompleta                                      y matriz real de
                                                  aplicaciones

  Conflictos en DPREFIX   Alto                    Locks, staging,
  compartido                                      operaciones
                                                  transaccionales y
                                                  pruebas

  Instaladores PKG        Alto                    Consentimiento,
  ejecutan scripts                                advertencias,
                                                  inspección y registro

  Traducción Metal/Vulkan Alto                    Validar
  insuficiente                                    features/extensiones y
                                                  no asumir equivalencia

  GPU híbrida mal         Medio/alto              Política centralizada y
  seleccionada                                    pruebas con equipos
                                                  híbridos

  Distintas               Alto                    Abstracción de host, CI
  distribuciones y libc                           y matriz de plataformas

  DRM/anticheat           Alto                    Documentar límites; no
                                                  prometer bypass ni
                                                  compatibilidad

  Crash del daemon o del  Alto                    Supervisión,
  entorno                                         recuperación de estado
                                                  y logs

  Crecimiento excesivo de Medio                   Crear crates según
  módulos                                         límites reales y evitar
                                                  abstracción prematura
  -----------------------------------------------------------------------

------------------------------------------------------------------------

## 15. MVP recomendado

El MVP no debe intentar ejecutar todas las aplicaciones ni soportar
todas las distribuciones. Debe demostrar un flujo vertical completo y
confiable.

### Alcance del MVP

1.  Linux x86-64 en una distribución objetivo inicial.
2.  Integración reproducible con una versión concreta de Darling.
3.  DPREFIX dedicado y compartido.
4.  DHI CLI con CPU, memoria, sistema, GPU PCI y Vulkan opcional.
5.  Inspector de bundles `.app`.
6.  Instalación inicial de bundles `.app` y ZIP.
7.  Catálogo SQLite.
8.  Compatibility Engine básico con arquitectura, dependencias y
    frameworks identificables.
9.  DEM por sesión con lanzamiento y supervisión de una aplicación de
    prueba cuya ejecución haya sido validada.
10. CLI para consultar aplicaciones, prefijo y procesos.
11. Logs y diagnósticos.
12. CI y pruebas de integración.

La interfaz visual completa, soporte amplio de PKG, AArch64,
optimización por categorías y múltiples formatos de paquete deben venir
después de validar el flujo central.

### Definición de terminado del MVP

El MVP se considera terminado cuando: - Puede compilarse desde un
entorno limpio siguiendo documentación. - Detecta hardware sin requerir
root. - Inicializa y valida un DPREFIX dedicado sin interferir con otro
prefijo. - Importa, inspecciona e instala una aplicación de prueba. -
Registra su metadata y estado. - Analiza requisitos y reporta
incertidumbres. - Lanza y supervisa una aplicación cuya compatibilidad
se haya demostrado. - Puede detenerla sin dejar el entorno en un estado
inconsistente. - Los fallos generan diagnósticos útiles. - Las pruebas
críticas pasan en CI.

------------------------------------------------------------------------

## 16. Primeros issues sugeridos

Para iniciar el repositorio, crear issues pequeños, verificables y
ordenados:

1.  **Bootstrap:** estructura del monorepo, licencia, README y
    convenciones.
2.  **Build:** workspace Cargo y configuración CMake mínima.
3.  **CI:** formato, Clippy, tests y build nativo.
4.  **Darling audit:** documentar versión, dependencias, DPREFIX y flujo
    real de ejecución.
5.  **DHI domain:** modelos y esquema JSON versionado.
6.  **DHI Linux:** detección de distribución, kernel y libc.
7.  **DHI CPU:** detección x86-64 y diseño AArch64.
8.  **DHI GPU PCI:** enumeración y normalización de dispositivos.
9.  **DHI Vulkan:** enumeración opcional de dispositivos y features.
10. **SPM lifecycle:** estados, validación y locks del prefijo.
11. **Bundle Inspector:** lectura segura de `Info.plist`.
12. **Mach-O inspection:** arquitectura y dependencias.
13. **App Library:** SQLite y migraciones.
14. **Installer staging:** extracción segura de ZIP y bundles.
15. **Compatibility report:** primera evaluación con evidencias.
16. **DEM prototype:** daemon y protocolo local mínimo.
17. **Vertical integration test:** importar, instalar, analizar, lanzar
    y cerrar una app de prueba.

------------------------------------------------------------------------

## 17. Criterio general de evolución

El proyecto debe crecer desde capacidades demostradas, no desde una
lista de funciones aspiracionales.

Para cada nuevo componente: 1. Definir el problema y el límite de
responsabilidad. 2. Comprobar si Darling o Linux ya resuelven parte del
problema. 3. Registrar decisiones importantes en un ADR. 4. Definir un
contrato mínimo. 5. Implementar una prueba pequeña. 6. Integrar el
componente con el flujo real. 7. Documentar capacidades, limitaciones y
plataformas probadas.

**Regla final:** ninguna característica debe anunciarse como compatible
hasta que exista evidencia reproducible. La plataforma debe preferir un
resultado explícito de "parcial" o "desconocido" antes que prometer una
compatibilidad que todavía no se ha demostrado.
