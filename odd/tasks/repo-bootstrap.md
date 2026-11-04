# Feature: repo-bootstrap

Estado: en curso · Rama: `feat/repo-bootstrap` · Especificación: `PLAN_DARLING_PLATFORM.md`

Primer corte de la plataforma: dejar un monorepo compilable, verificable en CI y
documentado, sin lógica funcional todavía. El objetivo de este corte es que
`cargo test`, `cargo clippy`, `cargo fmt --check` y el build de CMake pasen en
un entorno limpio.

## Specs

### S1 — Pedido original

> @PLAN_DARLING_PLATFORM.md vamos a empezar con la estructura para este proyecto

### S2 — Licencia

> GPL-3.0-or-later
>
> Más garantías explícitas (patentes, tivoización). Corre el riesgo de fricción
> si algún día se integra código GPL-2.0-only de Darling sin cláusula
> or-later.

El archivo `LICENSE` contiene el texto verbatim de GNU GPL v3 y la declaración
`GPL-3.0-or-later` aparece como SPDX en los `Cargo.toml` y en `README.md`.

### S3 — Alcance del primer corte: "Solo esqueleto compilable"

> Entrega:
> - Cargo.toml workspace + crates con lib.rs vacíos y test trivial
> - CMakeLists.txt que invoca cargo
> - .github/workflows/ci.yml (fmt, clippy, test, cmake build)
> - .gitignore, rustfmt.toml, .editorconfig
> - README.md, DEVELOPMENT.md, CONTRIBUTING.md, SECURITY.md
> - docs/architecture/overview.md + ADR-0001..0007
> - scripts/dev y scripts/ci mínimos
>
> Criterio: cargo test y cmake build pasan en limpio.

### S4 — Restricciones del plan

> La estructura es una dirección objetivo, no una obligación de crear todos los
> directorios vacíos desde el primer commit.

> Workspace inicial orientativo:
> `modules/platform-common`, `modules/hardware-intelligence`,
> `modules/shared-prefix`, `modules/compatibility-engine`,
> `modules/application-management/app-installer`,
> `modules/application-management/app-library`,
> `modules/application-management/bundle-inspector`,
> `services/execution-manager`

> En cada pull request: `cargo fmt --check`,
> `cargo clippy --workspace --all-targets -- -D warnings`,
> `cargo test --workspace`, Build CMake en configuración Debug.

> Decisiones iniciales que deben registrarse: ADR-0001 monorepo, ADR-0002
> DPREFIX compartido, ADR-0003 daemon por sesión, ADR-0004 Dedicated GPU First,
> ADR-0005 DHI separado de ejecución, ADR-0006 Darling como núcleo externo,
> ADR-0007 Rust + CMake.

No se crean directorios vacíos para `docs/compatibility/`, `docs/packaging/`,
`docs/development/`, `packaging/`, `tests/`, `integrations/`, `runtimes/` ni
`apps/application-center/`: se incorporan cuando exista la primera
implementación verificable que los necesite.

### S5 — Idioma de los artefactos

Código, identificadores, comentarios de código, mensajes de commit y salida de
las herramientas: inglés. Documentación de repositorio y ADR: español, porque la
convención vigente del proyecto la fija `PLAN_DARLING_PLATFORM.md`.

## Tasks

| ID | Spec | Entrega | Commits |
| --- | --- | --- | --- |
| T1 | S1, S5 | Higiene de repositorio, licencia GPL-3.0-or-later, documento ODD | `85994c3`, `bd03634` |
| T2 | S3, S4 | Workspace Cargo con los 8 crates del plan y CLI `dg` | `204dfe0` |
| T3 | S3 | CMake que orquesta cargo, instala `dg` y expone ctest | `0d15132` |
| T4 | S3 | README, DEVELOPMENT, CONTRIBUTING, SECURITY, código de conducta, CHANGELOG | `2289d87`, `e8d422f` |
| T5 | S3, S4 | `docs/architecture/overview.md`, `components.md`, `data-flow.md`, ADR-0001..0007 | `e4f17c1`, `6667ea4` |
| T6 | S3 | CI de GitHub Actions y scripts `dev`/`ci` | `2d64b0f` |
| — | S1 | Plan de referencia versionado en el repositorio | `d7c4968` |

## Log

### L1 — Pedido original (verbatim)

> @PLAN_DARLING_PLATFORM.md vamos a empezar con la estructura para este proyecto

### L2 — Decisiones del 2026-11-04

Licencia `GPL-3.0-or-later` y alcance "Solo esqueleto compilable", con los
criterios de salida transcriptos en S2 y S3.

### L3 — Desvíos conscientes respecto del plan

- Resolver de Cargo `"2"` según el snippet del plan §4.1, aunque la edición es
  2024 (que admite `"3"`). Se revisa cuando exista código que lo justifique.
- `Cargo.lock` se versiona: el plan §4 lo incluye en el árbol del repositorio y
  CI compila con `--locked`.
- No se crean `DarlingExternal.cmake` ni `Packaging.cmake` todavía: no tienen
  consumidor real (plan §12.2).

### L4 — Verificación (2026-11-04)

Ejecutado sobre `feat/repo-bootstrap` con árbol limpio:

| Comprobación | Comando | Resultado |
| --- | --- | --- |
| Formato | `cargo fmt --all --check` | OK |
| Lints | `cargo clippy --workspace --all-targets --locked -- -D warnings` | OK, sin warnings |
| Pruebas | `cargo test --workspace --locked` | 16 pruebas, 0 fallos |
| Build CMake | `cmake -S . -B build -G Ninja && cmake --build build` | OK |
| CTest | `ctest --test-dir build --output-on-failure` | 1/1 OK |
| Instalación | `cmake --install build --prefix /tmp/dp-install` | `bin/dg` instalado; `dg --version` → `dg 0.1.0` |
| Perfil release | `-DDP_CARGO_PROFILE=release` | OK |
| Árbol de fuentes intacto | build de CMake sin `./target` previo | no crea `target/` en el checkout |
| Compuertas locales | `scripts/ci/check.sh` | todas verdes |
| Entorno | `scripts/dev/env-check.sh` | todas las herramientas OK |
| Enlaces relativos | verificación de todos los `.md` | todos resuelven |
| YAML de CI | parseo de `.github/workflows/ci.yml` | válido, jobs `rust` y `native` |

Pendiente fuera de este corte: `cargo fmt`/`clippy` sobre un toolchain distinto al
local (lo cubre CI) y la ejecución del workflow en GitHub, que requiere remoto.
