# Changelog

Todos los cambios relevantes de este proyecto se documentan en este archivo.

El formato sigue [Keep a Changelog](https://keepachangelog.com/es-ES/1.1.0/) y el
proyecto usa [Versionado Semántico](https://semver.org/lang/es/).

## [Unreleased]

### Fixed

- `dg prefix start` crea el directorio padre del prefijo antes de invocar Darling,
  que solo crea el prefijo y fallaba con `No such file or directory` cuando la ruta
  contenedora no existía. `dg prefix validate` reporta ese caso con el check
  `prefix_parent`.
- El instalador de Darling descubre los paquetes recorriendo el árbol extraído (el
  release los pone en un subdirectorio fechado) e instala con `apt-get` en lugar de
  `dpkg -i`.

### Added

- Instalador de un comando (`scripts/install.sh`): instala el Darling pineado
  verificado por sha256 y el paquete `darling-platform`, elevando privilegios solo
  en los pasos que los necesitan.
- Empaquetado `.deb` reproducible de `darling-platform` y repositorio apt local con
  `Packages.gz`, `Release` y firma GPG opcional.
- Pruebas offline del paquete y del repositorio, integradas a CI.
- `ADR-0010`: instalador entregado como paquete `.deb` y contrato de privilegios.
- Pin de Darling `v0.1.20260608` (commit `e947f0d5…`) en `deps/darling.toml`,
  embebido en el binario y reportado por `dg prefix status`; script de instalación
  que verifica el sha256 del asset antes de instalar.
- ADR-0009: política de pin de Darling y verificación de instalación.
- Shared Prefix Manager: ciclo de vida del DPREFIX dedicado con estados
  persistidos, locks de lectura/escritura, validación por checks y recuperación de
  operaciones interrumpidas (`modules/shared-prefix`).
- Adaptador de Darling con los comandos verificados (`darling shell`,
  `darling shutdown`), que exporta `DPREFIX` al proceso hijo.
- CLI `dg prefix status|validate|start|stop`, con salida texto y `--json`, y
  rechazo de `stop` mientras haya instancias gestionadas activas.
- ADR-0008: contrato verificado del ciclo de vida del DPREFIX.
- Esqueleto del monorepo: workspace de Cargo con los ocho módulos del plan de
  arquitectura, cada uno con su responsabilidad y no-responsabilidad declaradas.
- CLI `dg` con `--version` y `--help`, y rechazo explícito de entrada
  desconocida con código de salida 2.
- Build de CMake que delega la compilación en Cargo, expone las pruebas a CTest e
  instala `dg` en el prefijo de instalación.
- Gates de CI y script local equivalente: `cargo fmt --check`,
  `cargo clippy -D warnings`, `cargo test`, build de CMake y `ctest`.
- Documentación de arquitectura: visión general, componentes, flujos de datos y
  ADR-0001 a ADR-0007.
- Documentación de repositorio: README, guía de desarrollo, contribución,
  seguridad y código de conducta.
- Licencia GPL-3.0-or-later.

<!-- TODO(repo-bootstrap): replace OWNER/REPO with the canonical remote once it
     exists. No URL is invented before that. -->

[Unreleased]: https://github.com/OWNER/REPO/compare/main...HEAD
