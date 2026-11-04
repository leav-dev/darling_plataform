# Instalación

Un solo comando instala todo: el Darling pineado (verificado por sha256) y el
paquete `darling-platform`, que provee `dg`.

## Requisitos

- Debian/Ubuntu (x86-64). El pin de Darling publica paquetes `.deb` para esta
  familia.
- Rust 1.85 o superior (`cargo`, `rustc`) si vas a compilar desde el checkout.
- `curl` o `wget`, `unzip`, `dpkg`, `apt-get`, `sudo`, `sha256sum`.
- Tu contraseña de `sudo`.

## Instalación en un comando

```bash
git clone https://github.com/leav-dev/darling_plataform.git
cd darling_plataforma
scripts/install.sh
```

Primero, sin ejecutar nada:

```bash
scripts/install.sh --dry-run      # imprime cada comando, incluidos los sudo
```

Qué hace, en orden:

| Paso | Qué hace | ¿Root? |
| --- | --- | --- |
| 1/5 preflight | Verifica sistema, arquitectura y herramientas | No |
| 2/5 privileged | Explica qué necesita root y pide la contraseña una vez (`sudo -v`) | Solo la credencial |
| 3/5 Darling | Descarga el release pineado, verifica el sha256 e instala con `dpkg -i` | Sí (`sudo`) |
| 4/5 darling-platform | Construye el `.deb` del checkout e instala con `apt-get install` | Solo el `apt-get` |
| 5/5 verificación | `dg --version`, `dg prefix validate`, `darling shell uname` → `Darwin` | No |

El script **no corre entero como root**: lo invoca tu usuario, eleva únicamente los
dos pasos que lo necesitan y no deja ninguna shell ni proceso privilegiado. Si lo
invocás como root, se niega y explica por qué.

### Opciones

| Opción | Efecto |
| --- | --- |
| `--dry-run` | Imprime cada comando y no ejecuta nada |
| `--yes` | No pregunta antes de cada paso (instalación desatendida) |
| `--no-darling` | Asume que el Darling pineado ya está instalado |
| `--from-deb ARCHIVO` | Instala un `.deb` ya construido en vez de compilar |
| `--help` | Muestra el uso |

## Qué queda instalado

| Artefacto | Dónde |
| --- | --- |
| Paquetes de Darling | Los del release pineado, vía `dpkg` (su binario es setuid root) |
| `dg` | `/usr/bin/dg`, dentro del paquete `darling-platform` |
| Datos de la plataforma | `~/.local/share/darling-platform/` (estado, lock, prefijo) |

El DPREFIX dedicado se crea en el primer `dg prefix start`, no durante la
instalación.

## Verificar

```bash
dg --version
dg prefix status
dg prefix validate
DPREFIX=/tmp/darling-check/dp darling shell uname    # debe imprimir: Darwin
```

La matriz de compatibilidad de cada Darling probado está en
[`compatibility/darling-pin.md`](compatibility/darling-pin.md).

## Actualizar

```bash
git pull
scripts/install.sh --no-darling      # reconstruye e instala el paquete del proyecto
```

Darling se actualiza cambiando el pin (`deps/darling.toml`) y volviendo a correr el
instalador; el procedimiento completo está en
[`compatibility/darling-pin.md`](compatibility/darling-pin.md).

## Desinstalar

```bash
sudo apt remove darling-platform
```

El prefijo `~/.local/share/darling-platform/prefix/` **no se borra**: contiene tu
entorno y las aplicaciones instaladas. Borrarlo es una decisión explícita:

```bash
rm -rf ~/.local/share/darling-platform
```

Los paquetes de Darling se quitan con `sudo apt remove darling` (o `purge`), y eso
sí afecta a cualquier otro uso de Darling en el sistema.

## Repositorio apt

Para usar el camino de repositorio en lugar de instalar un `.deb` suelto:

```bash
scripts/packaging/build-deb.sh
scripts/packaging/build-repo.sh              # repositorio local, sin firma
scripts/packaging/build-repo.sh --check      # re-verifica hashes y detecta drift
scripts/packaging/build-repo.sh --sign KEYID  # firma con gpg
```

Cada modo imprime la línea de `sources.list` que le corresponde, sin escribir en
`/etc`:

| Modo | Línea impresa | Qué implica |
| --- | --- | --- |
| Sin firma | `deb [trusted=yes] file://… local main` | apt no verifica nada: sirve para una prueba local, **nunca para publicar** |
| Con `--sign KEYID` | `deb [signed-by=…/darling-platform-archive-keyring.gpg] file://… local main` | apt verifica la firma del `Release` con el keyring exportado |

`--check` valida consistencia interna (`Packages` ↔ pool ↔ `Release`), no
autenticidad: quien pueda reescribir el pool puede reescribir el índice. La firma
y el hosting remoto llegan con la Fase 8.

## Problemas frecuentes

| Síntoma | Causa | Solución |
| --- | --- | --- |
| `error: run this as your normal user, not as root` | Se ejecutó con `sudo` | Corré `scripts/install.sh` sin `sudo` |
| `missing tools: …` | Falta `cargo`, `unzip` o similar | Instalá lo que indica el mensaje y reintentá |
| `sudo was refused` | Cancelaste la contraseña | Nada se instaló; reintentá cuando quieras |
| `dg is not on PATH` | `/usr/bin` fuera del PATH | Revisá el PATH o usá `/usr/bin/dg` |
| `darling shell uname` no dice `Darwin` | Darling instalado pero sin funcionar | Ver [`compatibility/darling-pin.md`](compatibility/darling-pin.md) |
| `Cannot create /ruta/dp: No such file or directory` | El **directorio padre** del prefijo no existe: Darling hace `mkdir` del prefijo, no de su ruta | `mkdir -p /ruta` antes de `darling`; `dg prefix start` ya lo crea solo |
