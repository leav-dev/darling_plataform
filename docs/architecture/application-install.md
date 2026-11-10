# Importación e instalación de aplicaciones

Estado: implementado (Fase 3, corte 1). Describe lo que hacen hoy
`dp app inspect`, `dp app install` y `dp app list`, con los formatos `.app` y
`.zip`, el flujo de staging y renombrado atómico, la política de reemplazo y el
catálogo SQLite.

Fuera de este corte: `.dmg`, `.pkg`, `.xip` (con `.pkg`, sus scripts), rollback de
una instalación completada y conversión de iconos `.icns`. Ver
[`ADR-0012`](decisions/ADR-0012-importacion-de-aplicaciones.md).

## 1. Comandos

Los tres comandos comparten las convenciones del CLI: `Response { code, stdout,
stderr }`, salida de texto o `--json`, y estas salidas:

| Código | Significado |
| --- | --- |
| 0 | La operación terminó; el resultado está en stdout |
| 1 | Fallo de ejecución; `error: <motivo>` en stderr y stdout vacío |
| 2 | Uso incorrecto; `error: <motivo>` más el bloque de uso en stderr |

`--json` imprime un único documento JSON compacto terminado en `\n`, con las
ausencias omitidas en lugar de `null` y los enums en `snake_case`.

### `dp app inspect <path>`

Lee el bundle sin ejecutarlo y sin tocar el prefijo, el catálogo ni el lock.
Texto:

```text
path: /ruta/Example.app
bundle_id: com.example.example
name: Example
display_name: Example App
version: 1.2.3
executable: Example
executable_path: /ruta/Example.app/Contents/MacOS/Example
icon_file: Example.icns
minimum_system_version: 10.13
architectures: x86_64
framework: none
resource: Contents/Resources/Example.icns
```

- `display_name`, `version`, `icon_file` y `minimum_system_version` solo aparecen
  si el bundle los declara con un valor no vacío.
- `architectures` une las arquitecturas con `,` en el orden del archivo, o
  imprime `none`.
- `framework:` y `resource:` se repiten por entrada, o imprimen `none`.
- Con `--json` se imprime `BundleMetadata`: `path`, `bundle_id`, `name`,
  `display_name`, `version`, `executable`, `executable_path`, `icon_file`,
  `minimum_system_version`, `architectures`, `frameworks`, `resources`. Las
  rutas de frameworks y recursos son relativas al bundle y no recursivas.

### `dp app install <path> [--replace]`

Resuelve la ubicación del prefijo, toma el **lock de escritura** del Shared
Prefix Manager (`PrefixBusy` si otra operación lo tiene), abre el catálogo y
delega en el instalador. Todo lo demás —staging, `<prefijo>/Applications`,
validaciones y las dos filas del catálogo— pertenece al instalador, que es quien
crea los directorios.

Texto:

```text
installed: /ruta/darling-platform/prefix/Applications/Example.app
installation_id: Example-1794182400-4711-0
format: app
bundle_id: com.example.example
name: Example
version: 1.2.3
architectures: x86_64
```

`format` es `app` o `zip`. `version` solo aparece si el bundle la declara. Con
`--json`:

```json
{"installation_id":"…","path":"…","format":"app","metadata":{…}}
```

donde `metadata` es el mismo `BundleMetadata` de `inspect`, con `path` y
`executable_path` reescritos para nombrar la copia instalada.

### `dp app list [--json]`

Abre el catálogo (lo crea vacío si no existe) y lista las aplicaciones por
nombre y luego por id. Texto:

```text
application: name=Example App bundle_id=com.example.example version=1.2.3 architecture=x86_64 status=installed path=/ruta/prefix/Applications/Example.app
```

Un catálogo vacío imprime `applications: none`. Con `--json` se imprime
`{"applications":[…]}` con las filas de `applications`. El historial de
instalaciones no se imprime en este corte.

## 2. Formatos: `.app` y `.zip` solamente

El formato lo decide la extensión, comparada sin distinguir mayúsculas; el
contenido no se inspecciona para adivinar otro formato. Un `.zip` no se prueba
como otra cosa, y un archivo sin extensión no es una fuente válida.

- `.app`: un bundle ya extraído, que se copia tal cual.
- `.zip`: un archivo que contiene el bundle. Solo los métodos *stored* y
  *deflate*, ambos en Rust puro (`zip` con `deflate-flate2-zlib-rs`), para no
  agregar una dependencia de sistema al `.deb`.

`.dmg`, `.pkg` y `.xip` se rechazan con un error que nombra la ruta y los
formatos soportados. Cada uno tiene su propio flujo: el DMG hay que montarlo, el
PKG ejecuta scripts de instalador —con efectos externos que ningún rollback
puede deshacer— y el `.xip` requiere la herramienta de expansión de Apple. Nada
de eso entra sin su propio diseño.

Un origen sin formato soportado **no es un intento de instalación** y no deja
fila en el historial: la columna `installations.format` guarda el vocabulario de
formatos y una fuente sin formato soportado no tiene valor que guardar.

## 3. Flujo, en orden

1. **Formato.** Se identifica por extensión (ver §2).
2. **Staging.** Se crea `<applications_dir>/.staging/<installation_id>`, en el
   mismo sistema de archivos que el destino, sobre lo que hubiera quedado de un
   intento anterior con el mismo id. `installation_id` debe ser un único
   componente de ruta; el CLI arma uno con `<nombre>-<segundos>-<pid>-<secuencia>`.
3. **Copia o extracción.** El `.app` se copia como árbol (ver §5) o el `.zip` se
   extrae validando cada entrada mientras se escribe.
4. **Un solo bundle.** Se busca recursivamente el único `.app` bajo el staging;
   no se desciende dentro de un bundle encontrado (un framework o un helper
   interno es parte de esa aplicación) y se ignora `__MACOSX`. Cero bundles y
   varios bundles son error, y el mensaje nombra lo que se encontró.
5. **Inspección.** El bundle se valida con el Bundle Inspector. Un bundle cuyos
   metadatos no se pueden probar nunca llega al prefijo.
6. **Destino.** Se deriva `<applications_dir>/<name>.app` del nombre del bundle
   (ver §4).
7. **Política.** Si el destino ya existe, se rechaza salvo que la petición pida
   reemplazar (ver §5).
8. **Un solo `rename`.** El bundle preparado se mueve a su destino con
   `std::fs::rename`. Sin copia: no existe un instante en que el destino tenga
   media aplicación.
9. **Catálogo.** La fila de `applications` y la de `installations` se escriben en
   **una sola transacción**. Si el catálogo rechaza algo, el sistema de archivos
   vuelve atrás: se borra la copia recién instalada y se restituye la reemplazada.
10. **Limpieza.** El staging se elimina en todos los caminos de salida, incluida
    la caída de la guarda; el área `.staging` se borra cuando queda vacía.

Ningún fallo deja una instalación parcial: el destino, el staging y el área se
retiran antes de devolver el error, y el error nombra el archivo, la entrada del
archivo comprimido o el valor declarado que lo causó.

## 4. Nombre del destino

`CFBundleName` es entrada no confiable y no tiene por qué ser un nombre de
directorio, así que se sanea en vez de usarse tal cual:

1. Se quita un sufijo `.app` final (el destino agrega el suyo).
2. Cada `/` y `\` se convierte en `-`.
3. Se recortan los espacios de los extremos.
4. Se rechaza el resultado si queda vacío, si es `.` o `..`, o si no es un único
   componente de ruta ordinario (lo que cubre bytes NUL, prefijos de unidad y
   separadores restantes).

El destino es siempre `<applications_dir>/<nombre saneado>.app`. Un bundle sin
`CFBundleName` usa el nombre del directorio sin `.app`, que es el respaldo que
macOS aplica. `CFBundleDisplayName` no se usa para el destino: se guarda como
`name` de la fila del catálogo cuando existe, y `CFBundleName` queda para la ruta.

## 5. Política de reemplazo

Por defecto, un destino ya ocupado es un error (`replace` en el mensaje): una
instalación existente no se sobrescribe nunca en silencio.

Con `--replace`, el intercambio es explícito:

1. La copia instalada se mueve a `.staging/<installation_id>-replaced`.
2. El bundle preparado se renombra al destino.
3. Si el segundo `rename` falla, la copia movida se restituye y el error lo dice.
4. Si la escritura del catálogo falla, se borra la copia nueva y se restituye la
   reemplazada: un fallo no deja una aplicación que nada pueda listar.
5. Con el catálogo escrito, la copia reemplazada se borra junto con el staging.

La fila de `applications` se **actualiza**, nunca se duplica: `replace_install`
hace un *upsert* por `id` y sobrescribe todas las columnas mutables, así que
ninguna versión, icono ni fecha de la fila anterior sobrevive. Las dos
instalaciones quedan como dos filas de `installations`, y el historial no se
pierde.

## 6. Catálogo

Un único archivo SQLite en el data home de la plataforma, fuera del DPREFIX para
que el overlay de Darling no lo oculte:
`<raíz de datos>/darling-platform/library.sqlite3`. La raíz de datos es
`XDG_DATA_HOME` si es absoluta y, si no, `~/.local/share`; el CLI deriva la ruta
del catálogo de la misma ubicación que resuelve el prefijo, así que las dos no
pueden discrepar. `dp app list` lo crea vacío si no existe; `dp app install`
también, antes de intentar nada.

### Migraciones

La versión del esquema vive en `PRAGMA user_version`, dentro del archivo, así que
un catálogo copiado conserva su versión. Cada paso corre en una transacción y
solo se aplican los pasos que faltan: abrir un catálogo al día no hace nada, y
uno escrito por una versión más nueva se rechaza con `SchemaTooNew` en lugar de
leerse con un esquema que este build no entiende. Los pasos son contiguos desde
1 y se agregan al final; uno ya aplicado no se reescribe. La versión actual es 1.

### Tablas

`applications`: una fila por aplicación instalada.

| Columna | Contenido |
| --- | --- |
| `id` | Clave primaria; el instalador guarda el bundle identifier |
| `bundle_id` | `CFBundleIdentifier`; indexado por la restricción única, no único por sí solo |
| `name` | `CFBundleDisplayName` si existe, si no `CFBundleName` |
| `version` | `CFBundleVersion`, opcional |
| `bundle_path` | Ruta del bundle instalado |
| `executable_path` | Ruta del ejecutable dentro del bundle |
| `icon_path` | Icono declarado, solo si el bundle lo trae; opcional |
| `architecture` | Arquitecturas separadas por `,`, o ausente |
| `installation_type` | `bundle` para un `.app`, `zip` para un `.zip` |
| `installed_at` | RFC 3339 UTC, provisto por el CLI |
| `installation_status` | `installed` |

Restricción única `(bundle_id, bundle_path)`: dos versiones instaladas en dos
rutas pueden compartir bundle id, pero un par repetido no.

`installations`: una fila por intento, con éxito o sin él.

| Columna | Contenido |
| --- | --- |
| `id` | Clave primaria; el `installation_id` del intento, único por intento |
| `application_id` | Dueño del intento, o `NULL` si no produjo fila de aplicación; `REFERENCES applications (id) ON DELETE SET NULL` |
| `origin` | De dónde salió el bundle, como lo vio el llamador |
| `format` | `app` o `zip` |
| `result` | `installed` o `failed` |
| `detail` | Motivo del fallo, cuando hay algo que decir |
| `occurred_at` | RFC 3339 UTC |

Índice `installations_by_application` sobre `(application_id, occurred_at, id)`.
Las claves foráneas se activan por conexión (`PRAGMA foreign_keys = ON`), así que
un historial no puede apuntar a una aplicación inexistente.

### Historial

El historial conserva los intentos fallidos: todo error posterior a la
aceptación del formato escribe una fila con `result = failed`, sin
`application_id` y con el mensaje del error en `detail`. Un fallo que ni siquiera
se puede registrar —porque `installation_id` se repitió, ya que es único por
intento— devuelve el error original acompañado de `FailureNotRecorded`, para que
un historial incompleto nunca se lea como otro tipo de fallo.

`Library::installations` devuelve todos los intentos, incluidos los que no
produjeron aplicación; `Library::histories(application_id)` devuelve solo los de
una aplicación. El CLI usa el primero en las pruebas y no imprime ninguno en
`dp app list` todavía.

## 7. Seguridad: la entrada no es confiable

`Info.plist`, las cabeceras Mach-O, las entradas del ZIP y las rutas del bundle
son entrada no confiable. Se valida antes de copiar y se rechaza todo lo que
pueda escribir fuera del destino.

### Rutas declaradas por el bundle

Cada ruta que el `Info.plist` declara se valida dos veces: léxicamente (nada
absoluto, nada con prefijo de unidad, ningún `..`) y físicamente (ningún symlink
cuyo destino canónico salga del bundle). Se aplica al ejecutable declarado, al
icono declarado, a los frameworks y a los recursos. El ejecutable además tiene
que existir, ser un archivo regular y tener una cabecera Mach-O legible; sin eso
no hay metadatos que confiar.

### Entradas del ZIP

Se rechaza la entrada, y el error nombra el archivo y la entrada:

| Entrada | Motivo |
| --- | --- |
| Ruta absoluta o con prefijo | Al unirla dejaría el área de staging |
| Componente `..` | Camina fuera del área |
| Byte NUL en el nombre | Truncaría la ruta en la llamada al sistema |
| Ruta que el lector no puede encerrar en el staging | No se puede probar el confinamiento |
| Enlace simbólico | Su destino se resuelve cuando se usa el archivo, después de toda comprobación |

Límites, comprobados contra lo declarado y contra los bytes realmente copiados
(una declaración no reemplaza un hecho):

- 200 000 entradas como máximo.
- 8 GiB (8 589 934 592 bytes) descomprimidos como máximo.

Los directorios se crean explícitamente y el padre de cada archivo se crea antes
del archivo, así que un ZIP sin entradas de directorio también se extrae.

### Enlaces simbólicos del `.app`

Un bundle real se apoya en enlaces (`Versions/Current`, `Resources/Current`, los
alias de frameworks), así que el bundle se copia como árbol y los enlaces se
recrean con su mismo texto, nunca se desreferencian. Un enlace se recrea solo
después de probar que su destino canónico sigue **dentro del bundle de origen**;
si resuelve afuera, se rechaza y el error nombra el enlace y su destino. En un
host que no puede representar enlaces simbólicos la copia se rechaza, en lugar de
instalar un bundle cuyos enlaces se volvieron archivos.

### Otros rechazos

- Un id de instalación que no es un único componente de ruta se rechaza antes de
  crear nada.
- Un nombre de bundle que no puede nombrar un directorio se rechaza (§4).
- Una entrada del `.app` que no es archivo regular, directorio ni enlace
  simbólico (socket, FIFO, nodo de dispositivo) se rechaza: copiarla podría
  bloquear para siempre.

## 8. Límites de este corte

- **No hay rollback implementado de una instalación completada.** El ADR-0012 lo
  diseña para el corte siguiente. Hoy sí hay deshacer *dentro* de una misma
  instalación: si el catálogo rechaza las filas, la copia nueva se borra y la
  reemplazada se restituye. Pero cuando el catálogo acepta, la fila ya está
  actualizada y la copia anterior se borra: no queda de dónde volver.
- **`application_id` es el bundle identifier**, así que hay una fila por bundle
  id. Dos bundles con el mismo identificador en rutas distintas chocan en el
  catálogo salvo instalación con `--replace`, que actualiza la fila existente;
  esto va más allá de la restricción única `(bundle_id, bundle_path)` del
  esquema, que por sí sola admitiría dos rutas.
- **El icono se registra, no se convierte.** `icon_path` apunta al archivo dentro
  del bundle instalado y solo se guarda si existe y su destino canónico sigue
  dentro del staging; no se copia a un directorio de iconos del data home y no
  hay conversión `.icns`, porque todavía no hay interfaz que consuma iconos.
- **Las reglas de enlaces del `.app` solo están cubiertas por pruebas
  `#[cfg(unix)]`.** En un host sin enlaces simbólicos la rama que rechaza la
  copia existe, pero no tiene prueba propia en este corte.
- `dp app list` no imprime el historial, no desinstala y no lanza nada. El
  análisis de compatibilidad tampoco corre: el instalador no decide si la
  aplicación puede ejecutarse.
