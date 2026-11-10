# Guía de desarrollo

Este documento describe cómo se compila, se prueba y se modifica el repositorio.
Las decisiones de arquitectura y producto viven en
[`PLAN_DARLING_PLATFORM.md`](PLAN_DARLING_PLATFORM.md) y en los
[ADR](docs/architecture/decisions/); acá están solo los detalles operativos.

## Entorno

| Herramienta | Mínimo | Probado con | Notas |
| --- | --- | --- | --- |
| Rust | 1.88 (edición 2024) | 1.96.1 | `rustfmt` y `clippy` instalados |
| CMake | 3.20 | 3.28.3 | Coordina el build y la instalación |
| Ninja | — | 1.11.1 | Opcional; si falta, CMake usa Makefiles |
| Git | — | 2.43.0 | |

Comprobación automática, sin instalar nada:

```bash
scripts/dev/env-check.sh
```

Ningún paso de compilación, prueba o escaneo de hardware requiere root. Si algo
lo pide, es un defecto, no una instrucción a seguir.

## Estructura del workspace

| Crate | Ruta | Rol |
| --- | --- | --- |
| `dp-platform-common` | `modules/platform-common` | Identificadores, errores y diagnósticos compartidos |
| `dp-hardware-intelligence` | `modules/hardware-intelligence` | DHI: descubre y describe, no decide |
| `dp-shared-prefix` | `modules/shared-prefix` | DPREFIX: ubicación, estado persistido, locks, adaptador `darling` |
| `dp-compatibility-engine` | `modules/compatibility-engine` | Requisitos frente a capacidades y evidencias |
| `dp-app-installer` | `modules/application-management/app-installer` | Staging, validación e instalación |
| `dp-app-library` | `modules/application-management/app-library` | Catálogo SQLite y migraciones |
| `dp-bundle-inspector` | `modules/application-management/bundle-inspector` | Metadatos de bundles sin ejecutarlos |
| `dp-execution-manager` | `services/execution-manager` | Daemon de sesión y procesos supervisados |
| `dp-cli` | `apps/cli` | Binario `dp` |

Cada crate documenta en su `lib.rs` qué hace **y qué no hace**; ese texto es el
contrato que se revisa al agregar la primera implementación. No agregues código a
un módulo antes de que exista una tarea que lo justifique.

### Dependencias del Shared Prefix Manager

| Dependencia | Rol | Por qué |
| --- | --- | --- |
| `rustix` (`fs`, `process`) | `flock` y uid | Evita FFI a mano y `unsafe` en el crate; mantenida y con licencia permisiva |
| `serde` + `serde_json` | Registro de estado y salida `--json` | El estado persistido y los reportes son contratos versionados |
| `tempfile` (solo tests) | Directorios aislados | Las pruebas no deben tocar el prefijo real ni `$HOME` |

Ver `docs/architecture/decisions/ADR-0008-ciclo-de-vida-prefix.md` para los
comandos de Darling que el adaptador puede usar.

## Comandos

```bash
# Formato, lints y pruebas
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked

# CLI
cargo run --bin dp -- --version
cargo run --bin dp -- --help
cargo run --bin dp -- prefix status
cargo run --bin dp -- prefix validate --json

# Build completo por CMake (Cargo compila dentro del árbol de build)
cmake -S . -B build -G Ninja
cmake --build build
ctest --test-dir build --output-on-failure
cmake --install build --prefix /tmp/dp-prefix

# Todas las compuertas de CI, en orden
scripts/ci/check.sh

# Darling pineado (ver docs/compatibility/darling-pin.md)
scripts/dev/install-darling-pin.sh --dry-run
```

Opciones relevantes:

| Variable / opción | Default | Efecto |
| --- | --- | --- |
| `-DDP_CARGO_PROFILE=` | `debug` | Perfil de Cargo (`debug` o `release`) |
| `-DDP_CARGO_TARGET_DIR=` | `<build>/cargo` | Directorio de artefactos de Cargo |
| `-DCMAKE_INSTALL_BINDIR=` | `bin` | Destino de los binarios instalados |
| `DP_BUILD_DIR` | `build` | Solo para `scripts/ci/check.sh` |
| `DPREFIX` | `~/.local/share/darling-platform/prefix` | Prefijo dedicado; el convencional `~/.darling` se rechaza |
| `XDG_DATA_HOME` | `~/.local/share` | Raíz del directorio de datos de la plataforma |
| `DP_DARLING` | `darling` | Binario de Darling que invoca el adaptador |
| `XDG_CACHE_HOME` | `~/.cache` | Caché de la descarga verificada del pin de Darling |

El build de CMake usa `--target-dir` dentro del árbol de build: **nunca** escribe
en `./target` ni en el árbol de fuentes. Si una regla nueva rompe eso, es un
defecto.

## Cuándo corre CI

| Evento | ¿Corre? | Por qué |
| --- | --- | --- |
| PR abierto o actualizado | Sí | Es el gate que importa: el árbol que se va a mergear |
| Push a `main` | Sí | Confirmar que `main` quedó verde después del merge |
| Push a una rama de feature | No | La cubre el evento de PR; correr ambos probaría el mismo árbol dos veces |
| Cambio de solo documentación | No | `paths-ignore` excluye `**/*.md`, `docs/**`, `odd/**` y `LICENSE` |
| Manual (`workflow_dispatch`) | Sí | Para una rama sin PR |

Si algún día se exige un check como *required* en la protección de rama, un PR de
solo documentación quedaría bloqueado: `paths-ignore` lo deja **pendiente**, no
verde. En ese momento hay que quitar el filtro o dejar de exigir el check.

## Flujo de trabajo

1. Abrí o buscá un issue que describa el cambio y su criterio de salida.
2. Trabajá en una rama `feat/…`, `fix/…`, `docs/…` o `ci/…`; `main` queda verde.
3. Commits pequeños y revisables: un cambio de comportamiento por commit, con sus
   pruebas y su documentación.
4. Antes de pedir revisión: `scripts/ci/check.sh`.
5. El PR incluye qué problema resuelve, alcance, decisiones, pruebas ejecutadas y
   riesgos conocidos.

Si el trabajo abarca varias sesiones o entregables, se registra como documento de
feature en `odd/tasks/<feature>.md` con sus specs, tareas y bitácora. Es el
insumo que leen los agentes y la verificación; no un resumen posterior.

## Agregar un crate

1. Creá `Cargo.toml` heredando del workspace:

   ```toml
   [package]
   name = "dp-nombre"
   description = "Una línea: qué hace el crate."
   version.workspace = true
   edition.workspace = true
   rust-version.workspace = true
   license.workspace = true
   authors.workspace = true
   publish.workspace = true
   ```

2. Agregá la ruta a `members` en el `Cargo.toml` raíz.
3. Escribí `src/lib.rs` con la responsabilidad y la no-responsabilidad del
   módulo, y una prueba que verifique que la descripción del manifiesto no está
   vacía (es la convención que mantiene los límites explícitos).
4. Documentá el crate en la tabla de este archivo y en `README.md` si cambia el
   mapa del repositorio.
5. `cargo fmt --all && scripts/ci/check.sh`.
6. Commit `feat(<área>): …` o `build(<área>): …`.

No agregues un crate por archivo ni una abstracción sin consumidor real.
`Cargo.lock` se versiona; CI compila con `--locked`.

## Convenciones de código

- **Idioma**: código, nombres, comentarios de código, mensajes de commit y salida
  de herramientas en inglés; documentación del repositorio y ADR en español.
- **Errores explícitos**: tipados, con subsistema, operación y categoría estable.
  Nada de errores genéricos que obliguen a interpretar cadenas.
- **Sin estado global mutable** y sin `unsafe` sin justificación escrita.
- **Sin dependencias sin justificar** mantenimiento, licencia y necesidad.
- **Pruebas al corregir un bug**, siempre.
- No acoples lógica de negocio a D-Bus, SQLite ni a la interfaz.

## Commits

[Conventional Commits](https://www.conventionalcommits.org/es/v1.0.0/) en inglés,
con el motivo en el cuerpo cuando no sea obvio:

```text
build(cmake): install the dp binary into the prefix

Explain why the change exists, not what the diff already shows.
```

Tipos usados: `feat`, `fix`, `docs`, `build`, `ci`, `refactor`, `test`, `chore`.

## Decisiones

Una decisión que cambia una regla del plan se registra en
`docs/architecture/decisions/`:

1. Creá `ADR-000N-titulo-corto.md` a partir del formato de los existentes.
2. Estado `propuesta` mientras se discute, `aceptada` al integrarse.
3. Si reemplaza a otra, marcá la anterior como `reemplazada` y enlazá ambas.
4. Actualizá el plan y la documentación relacionada en el mismo cambio.

## Problemas frecuentes

| Síntoma | Causa habitual | Solución |
| --- | --- | --- |
| `error: no se encontró el componente 'rustfmt'` | Toolchain sin componentes | `rustup component add rustfmt clippy` |
| CMake sigue usando un perfil viejo | Caché de `build/` | Borrá el directorio, o pasá `-DDP_CARGO_PROFILE=` de nuevo |
| `ctest` no encuentra pruebas | No se configuró CMake | `cmake -S . -B build` antes de `ctest --test-dir build` |
| El build escribe en `./target` | Invocación directa de `cargo` | Es esperable fuera de CMake; `.gitignore` lo cubre |
| `dp prefix start` falla con `DarlingUnavailable` | No hay binario `darling` | Instalá Darling o apuntá `DP_DARLING` al ejecutable |
| `dp prefix start` dice `PrefixBusy` | Otra operación tiene el lock | Esperá a que termine, o revisá `dp prefix status` |
| `dp prefix stop` rechaza | Hay instancias gestionadas activas | Cerrá esas instancias: `darling shutdown` las mataría |
