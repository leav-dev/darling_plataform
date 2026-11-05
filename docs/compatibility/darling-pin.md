# Darling pineado

La plataforma se desarrolla y se prueba contra **una** versión de Darling. Este
documento explica cuál, cómo se instala de forma verificable y qué hacer cuando
haga falta subir de versión. La decisión está en
[`ADR-0009`](../architecture/decisions/ADR-0009-pin-de-darling.md).

## Versión fijada

| Campo | Valor |
| --- | --- |
| Release | `v0.1.20260608` (publicada 2026-06-09) |
| Commit | `e947f0d5a3c6bba27e3631d9bf0da2a9840e6894` |
| Canal | snapshot fechado de `master`, publicado como release estable |
| Paquetes | `debs_20260608.zip` · 118.018.438 bytes · `sha256 27469ef3…eca1fd` |
| Fuentes | `darling-source.tar.xz` · 505.807.588 bytes · `sha256 4fc9eb9a…b58df7` |
| Parches propios | ninguno |

La fuente de verdad es [`deps/darling.toml`](../../deps/darling.toml). Se embebe en
el binario: `dp prefix status` reporta la versión con la que fue construido.

## Instalación verificada

```bash
scripts/dev/install-darling-pin.sh --dry-run   # muestra URL, hash y destino
scripts/dev/install-darling-pin.sh             # descarga y verifica; no instala
scripts/dev/install-darling-pin.sh --yes       # instala con dpkg (pide sudo)
```

Qué garantiza el script:

1. Descarga el asset **del tag pineado**, no de la rama.
2. Comprueba el sha256 registrado en el pin y **aborta** si no coincide.
3. Sin `--yes` no instala nada: imprime el `dpkg -i` exacto para que lo ejecutes vos.
4. En sistemas sin `dpkg` no intenta nada raro: señala el archivo de fuentes como
   alternativa.

El asset extrae en un subdirectorio fechado (`debs_20260609/`) y los paquetes de
adentro llevan la versión del día en que se construyeron
(`0.1.20260609~noble`), que puede diferir del tag: el tag marca el commit, el
paquete marca el build. El instalador descubre los `.deb` recorriendo el árbol
extraído e instala con `apt-get` (no `dpkg -i`) porque son 21 paquetes y apt
resuelve el orden y las dependencias.

Comprobación posterior:

```bash
mkdir -p /tmp/darling-check
DPREFIX=/tmp/darling-check/dp darling shell uname    # debe imprimir: Darwin
```

El `mkdir -p` no es opcional si el prefijo va dentro de una ruta que todavía no
existe: Darling crea **el prefijo**, no sus directorios padre, y falla con
`Cannot create <ruta>: No such file or directory`. El Shared Prefix Manager lo
resuelve por vos: `dp prefix start` crea solo el padre y deja que Darling cree el
prefijo.

> **`darling` es setuid root.** El binario aborta si no corre con privilegios
> efectivos de root (`missingSetuidRoot` en `src/startup/darling.c`). La plataforma
> no requiere root para sus operaciones, pero el comando que invoca sí lo necesita
> del lado de Darling. Ver [`SECURITY.md`](../../SECURITY.md).

## Qué no hace el pin

- **No detecta la versión instalada.** Darling no expone un comando que la reporte;
  inventar una heurística (fecha del binario, presencia de un archivo) daría falsos
  positivos. El pin se verifica al instalar, que es donde el dato existe.
- **No impide instalar otro Darling.** Si hay una versión distinta en el sistema,
  `dp prefix start` la usará. Los reportes de compatibilidad registran la versión
  esperada; agregar la detectada requiere que upstream la exponga.
- **No garantiza compatibilidad de aplicaciones.** "Estable" es una propiedad de la
  release de Darling, no de una app concreta.

## Matriz de compatibilidad

Todavía vacía: no se ejecutó Darling en ninguna máquina de desarrollo. Se completa
en la Fase 0 con evidencia reproducible.

| Darling | Distribución | Kernel | Resultado de `start`/`stop` | Notas |
| --- | --- | --- | --- | --- |
| `v0.1.20260608` | — | — | sin probar | pendiente de Fase 0 |

## Subir de versión

1. Actualizá `deps/darling.toml`: `version`, `commit`, `published_at`, nombres y
   sha256 de los assets.
2. Actualizá la prueba `pin_matches_the_release_this_platform_was_reviewed_against`
   en `modules/shared-prefix/src/pin.rs`; si no coincide, el build falla.
3. Actualizá las tablas de este documento y del ADR-0009 si cambia una decisión.
4. Corré `scripts/dev/install-darling-pin.sh` y la matriz de compatibilidad.
5. Anotá en el `CHANGELOG.md` el salto y, si algo se rompió, el diagnóstico.

## Diagnóstico

| Síntoma | Causa probable | Qué hacer |
| --- | --- | --- |
| `DarlingUnavailable` al hacer `start` | No hay binario `darling` | Instalá el pin o apuntá `DG_DARLING` |
| El hash no coincide al descargar | Descarga incompleta o asset reemplazado | Borrá el archivo del caché y reintentá; si persiste, revisá el release |
| `missingSetuidRoot` | El binario perdió el bit setuid | Reinstalá los paquetes del pin |
| `dp prefix status` muestra otro Darling | Instalación manual ajena al pin | Verificá con `dpkg -l` y reinstalá el pin |
