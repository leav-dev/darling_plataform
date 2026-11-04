# Feature: installer

Estado: en curso · Rama: `feat/installer` (sobre `feat/darling-pin`) · Fase 8 del plan

Un solo comando instala todo: Darling pineado, verificado con su sha256, más el
paquete `darling-platform` que provee `dg`. La instalación pasa por `dpkg`/`apt`,
no por copiar archivos a mano, para que actualizar y desinstalar sean operaciones
del sistema.

## Specs

### S1 — Pedido original

> vamos manjear un instalador, quiero que con un solo comadno puedo instalr todo

### S2 — Forma y privilegios (decidido por el usuario)

> Paquete .deb propio + repositorio

> se quiere mostrar la notificacion que se necesita usar privilegios de root, si,
> el sistema corre un comadno para ejecutar lo querido en root, al finalizar se
> retira los permisos, o se puede solicitar los permisos y que la persona digite la
> contraseña para elevar los privilegios

Consecuencias que el instalador debe cumplir:

- Avisa **antes** de elevar que se necesita root y **para qué** cada paso.
- Se ejecuta como usuario normal; eleva solo el paso que lo requiere, con `sudo`
  pidiendo la contraseña, y no deja una shell ni un proceso con privilegios.
- Si se lo invoca como root, se niega y explica: correr todo como root contradice
  el pedido de retirar privilegios al terminar.
- `--dry-run` muestra cada comando, incluidos los `sudo`, sin ejecutar nada.

### S3 — Envoltorio de paquete

El paquete del proyecto se llama `darling-platform`, provee `/usr/bin/dg` y declara
`Depends: libc6`. El número de versión sale del `Cargo.toml` del workspace
(`0.1.0`) con revisión de empaquetado (`0.1.0-1`), y una prueba falla si divergen.

El repositorio apt es local y reproducible (`scripts/packaging/build-repo.sh`):
`dpkg-scanpackages` genera `Packages.gz`, con firma GPG opcional. Publicar un
repositorio remoto firmado queda fuera de este corte y es el camino de la Fase 8.

### S4 — Restricciones del plan

> No instalar drivers ni modificar configuraciones del host automáticamente.

> Solicitar consentimiento antes de ejecutar instaladores que puedan lanzar
> scripts o realizar cambios.

> los paquetes no dependen de rutas locales de desarrollo y las
> instalaciones/actualizaciones se pueden reproducir en entornos limpios

## Tasks

| ID | Spec | Entrega | Commits |
| --- | --- | --- | --- |
| T1 | S3 | `packaging/deb` + `scripts/packaging/build-deb.sh`: `.deb` reproducible con `dg` | `3cb87e5` |
| T2 | S3 | `scripts/packaging/build-repo.sh`: repositorio plano con `Packages.gz`, `Release`, `--check` y firma opcional | `08c2a6c` |
| T3 | S1, S2 | `scripts/install.sh`: un comando, preflight, elevación por paso, verificación final | `f326f9f` |
| T4 | S3, S4 | Pruebas offline del paquete y del repositorio, integradas a CI | `7b15ae6` |
| T5 | S1, S2 | `docs/installation.md`, README, CHANGELOG y ADR-0010 | `docs(repo)` |

## Log

### L1 — Pedido original (verbatim)

> vamos manjear un instalador, quiero que con un solo comadno puedo instalr todo

### L2 — Decisiones

- Forma: paquete `.deb` del proyecto más repositorio apt, no un script que copia
  archivos. Habilita `apt install`, actualización y purga reales.
- Privilegios: el instalador corre sin privilegios y eleva por paso con `sudo`;
  avisa qué necesita root y por qué antes de pedir la contraseña.
- El repositorio remoto firmado (GPG + hosting) queda para la Fase 8; este corte
  entrega el repositorio local reproducible, que es lo que se puede verificar hoy.

### L3 — Límites explícitos

- No se empaqueta Darling: se instala desde su release pineado, con su hash. Un
  paquete propio de Darling duplicaría trabajo de upstream y su licencia.
- No se agregan repositorios de terceros al sistema.
- No se ejecuta `apt-get upgrade` ni se toca nada fuera de los dos paquetes.
- El desinstalador es el del sistema: `sudo apt remove darling-platform`. El
  DPREFIX con las aplicaciones **no** se borra automáticamente.

### L4 — Verificación (2026-11-04)

| Comprobación | Resultado |
| --- | --- |
| `tests/packaging/test_packaging.sh` | 40 comprobaciones, 0 fallos |
| `dpkg-deb --info dist/darling-platform_0.1.0-1_amd64.deb` | `Package: darling-platform`, `Version: 0.1.0-1`, `Architecture: amd64`, `Installed-Size: 820`, `Section: utils`, `Depends: libc6 (>= 2.39)` |
| Contenido del paquete | solo `usr/bin/dg` (0755), `copyright`, `changelog.gz`; control con `control` y `md5sums` |
| Rebuild | byte-idéntico (`6ab2e1dc92da0ddf…`) con `SOURCE_DATE_EPOCH` fijado al último commit |
| Repositorio | `pool/` + `dists/local/{Release,main/binary-amd64/{Packages,Packages.gz}}`; `--check` acepta y rechaza un paquete alterado |
| Firma | `--sign` produce `Release.gpg` y exporta el keyring; sin clave falla con mensaje accionable |
| `scripts/install.sh --dry-run` | exit 0; imprime los cinco pasos, los dos `sudo` y la verificación final |
| `scripts/ci/check.sh` | verde completo, con la suite de empaquetado como última compuerta |

### L5 — Lo que se agregó a la interfaz del paquete

- `Installed-Size` lo calcula el script: `dpkg-deb` no lo sintetiza (lo hace
  debhelper) y una comprobación lo exige distinto de cero.
- El repositorio sin firma imprime `[trusted=yes]`, no `[signed-by=…]`: apt no puede
  verificar un `Release` sin firmar y afirmar lo contrario sería mentir. Con `--sign`
  imprime el `signed-by` con el keyring exportado.
- La arquitectura del repositorio sale de `dpkg --print-architecture` y se compara
  con cada `.deb`: un archivo mal etiquetado falla en vez de publicarse.

### L6 — Pendiente y límites

- **No se ejecutó** la instalación real: `scripts/install.sh` sin `--dry-run` instala
  paquetes del sistema (uno setuid root) y necesita la contraseña del usuario. El
  ensayo completo queda para quien lo corra; el camino está verificado hasta el
  borde de `sudo`.
- Repositorio remoto firmado y hosting: Fase 8.
- `Depends: libc6 (>= 2.39)` ata el paquete a Ubuntu 24.04; se revisa al ampliar la
  matriz de distribuciones.
- La suite de empaquetado necesita el registro de Cargo tibio (`cargo fetch`) en una
  máquina sin red.
