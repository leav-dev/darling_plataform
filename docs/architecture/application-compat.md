# Análisis de compatibilidad

Estado: implementado (Fase 4, corte 1). Describe lo que hace hoy `dp app compat
<path> [--json]`, la forma del reporte, el perfil de capacidades contra el que
compara y lo que el análisis todavía no mira.

El análisis es estático y **es evidencia, no una garantía**. El plan lo dice
(`PLAN_DARLING_PLATFORM.md` §5.7: «No convertir un análisis estático favorable en
una garantía de ejecución») y el contrato del crate lo repite: un resultado
favorable no promete que la aplicación se ejecute. La compatibilidad real exige
pruebas.

Fuera de este corte: análisis de símbolos y APIs, requisitos gráficos, de audio y
de entrada, DRM/anticheat, análisis dinámico, la matriz inicial de aplicaciones
reales y cualquier forma de gating sobre `dp app install`. Ver
[`ADR-0013`](decisions/ADR-0013-motor-de-compatibilidad.md).

## 1. El comando

`dp app compat <path> [--json]` inspecciona el bundle —metadatos del bundle y load
commands del ejecutable—, compara lo que declara con el perfil embebido y publica
un reporte con una línea de evidencia por requisito.

| Código | Significado |
| --- | --- |
| 0 | El veredicto es `compatible` o `partial` |
| 1 | El veredicto es `not_compatible` o `unknown`, o el análisis falló |
| 2 | Uso incorrecto: falta la ruta, sobra un argumento o la ruta parece una opción |

`not_compatible` y `unknown` comparten el código 1 porque ninguno de los dos es
una luz verde para un script: uno es un fallo conocido y el otro es ausencia de
evidencia. Un análisis fallido imprime `error: <motivo>` en stderr y deja stdout
vacío; un veredicto con código 1 sí imprime el reporte completo en stdout.

El comando es de solo lectura de punta a punta:

- **Nunca resuelve el prefijo.** No construye `PrefixLocation`, no abre el
  catálogo, no toma el lock y no lee ni escribe el estado del ciclo de vida.
- **No escribe nada.** Ni en el bundle ni en el data home: el prefijo, el archivo
  de catálogo y el lock siguen sin existir después de la llamada.
- **No condiciona la instalación.** `dp app install` no consulta el motor, y un
  veredicto `not_compatible` no bloquea nada.
- **No ejecuta el binario ni resuelve ningún install name.** Sólo lee el bundle y
  los load commands de su ejecutable.
- **Es determinista.** La arquitectura del host la aporta el CLI
  (`std::env::consts::ARCH`); el motor no lee el host, así que el mismo bundle y
  el mismo perfil producen siempre el mismo reporte.

### Salida de texto

Tres líneas de encabezado y una línea por requisito, con la forma
`requirement: <estado> <tipo> <sujeto>: <detalle>`:

```text
verdict: unknown
analyser_version: 1
profile_version: 1
requirement: satisfied architecture x86_64: the executable ships a x86_64 slice and the host architecture is 'x86_64', so this slice runs natively
requirement: unknown dynamic_library /usr/lib/libz.1.dylib: the capability profile (version 1) records '/usr/lib/libz.1.dylib' as unknown: nobody has verified it against the pinned Darling build; the profile notes: No hay ninguna prueba registrada contra el pin.
requirement: unknown framework AppKit: the capability profile (version 1) records 'AppKit' as unknown: nobody has verified it against the pinned Darling build; the executable references '/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit'; the profile notes: No hay ninguna prueba registrada contra el pin; base de casi toda aplicación con interfaz.
requirement: satisfied minimum_os 10.13: the application declares macOS '10.13', which the profile's macOS 11.6 (Darwin 20.6.0) covers
```

- Los nombres de estado, tipo y veredicto son los mismos de `--json`
  (`snake_case`), tomados de la misma serialización para que texto y JSON no
  puedan divergir.
- `schema_version` sólo aparece en `--json`.
- Un reporte sin requisitos imprime `requirement: none` en lugar de una lista
  vacía.

### Salida JSON

`--json` imprime un único documento compacto terminado en `\n`. Aquí aparece
indentado para poder leerlo:

```json
{
  "schema_version": "1.0",
  "verdict": "unknown",
  "analyser_version": "1",
  "profile_version": "1",
  "requirements": [
    {
      "kind": "minimum_os",
      "subject": "none",
      "status": "unknown",
      "detail": "neither the bundle metadata nor the executable's load commands declare a minimum macOS version, so nothing can be compared with the profile's macOS 11.6"
    }
  ]
}
```

Las colecciones siempre están presentes, una ausencia se omite en lugar de
serializarse como `null` y los enums van en `snake_case`, igual que el resto de
los reportes de la plataforma.

## 2. Forma del reporte

| Campo | Qué es |
| --- | --- |
| `schema_version` | Versión de la forma JSON; hoy `1.0`. Agregar campos la mantiene, quitar o resignificar uno sube el mayor |
| `verdict` | Resultado general, derivado del peor requisito |
| `analyser_version` | Versión de las reglas de análisis; hoy `1`. Cambiar el significado de un veredicto o de una comprobación la sube |
| `profile_version` | `profile_version` del perfil con el que se comparó |
| `requirements` | Una entrada por evidencia, en orden estable |

Cada requisito tiene cuatro campos:

| Campo | Qué es |
| --- | --- |
| `kind` | Tipo de evidencia: `architecture`, `dynamic_library`, `framework` o `minimum_os` |
| `subject` | Qué se comprobó: una arquitectura, un nombre de framework, un install name o una versión declarada. Nunca está vacío |
| `status` | `satisfied`, `advisory`, `unknown` o `unsatisfied` |
| `detail` | Por qué, con las rutas, versiones y notas del perfil en las que se apoya el resultado |

Los cuatro veredictos:

| Veredicto | Cuándo |
| --- | --- |
| `compatible` | Todos los requisitos se comprobaron y se satisficieron |
| `partial` | Todo se satisface, o sólo con advertencias documentadas |
| `unknown` | Nada falla, pero algo no se puede decidir |
| `not_compatible` | Al menos un requisito está definitivamente insatisfecho |

La regla de severidad es la del peor requisito:

1. cualquier `unsatisfied` da `not_compatible`;
2. si no, cualquier `unknown` da `unknown`;
3. si no, cualquier `advisory` da `partial`;
4. sólo una lista no vacía con todo `satisfied` da `compatible`.

Una lista de requisitos vacía es `unknown`, nunca `compatible`: si no se
comprobó nada, no se sabe nada. Y un `unknown` no se presenta ni como compatible
ni como fallo.

El orden también es parte del contrato: primero la arquitectura, después las
bibliotecas dinámicas por ruta, después los frameworks por nombre y al final la
versión mínima de macOS. Dentro de cada bloque, las arquitecturas siguen el orden
de los slices y rutas y nombres van ordenados.

## 3. Qué se comprueba hoy

| Tipo | Se compara con | Evidencia que queda en el reporte |
| --- | --- | --- |
| `architecture` | La arquitectura del host | Los slices del ejecutable y el nombre del host |
| `dynamic_library` | La tabla `[[dylibs]]` del perfil | El install name declarado y la entrada del perfil |
| `framework` | La tabla `[[frameworks]]` del perfil | El nombre del framework y la referencia que lo prueba |
| `minimum_os` | `macos_version` del perfil | La versión declarada y la línea base del perfil |

### 3.1 Arquitecturas

Se comparan los slices del ejecutable que reporta el Bundle Inspector con la
arquitectura del host. Los nombres Mach-O se traducen al vocabulario del host
antes de comparar, sin distinguir mayúsculas:

| Slice Mach-O | Nombre en el reporte |
| --- | --- |
| `arm64` | `aarch64` |
| `x86_64` | `x86_64` |
| `arm` | `arm` |
| `i386` | `i386` |
| `powerpc` | `powerpc` |
| cualquier otro `cputype` | `unknown-<cputype en hex>`, sin nombre propio |

Se emite **un requisito `satisfied` por cada slice que el host puede correr**. Si
ningún slice coincide, se emite un único requisito `unsatisfied` cuyo sujeto
enumera los slices del binario —o `none`— y cuyo detalle dice que ninguna slice
corre en este host. **No hay traducción**: una aplicación arm64 sobre un host
x86_64 es `not_compatible` aunque macOS ofrezca Rosetta, porque la plataforma no
traduce binarios en este corte.

### 3.2 Bibliotecas dinámicas

Se leen los cuatro load commands que declaran una dependencia —`LC_LOAD_DYLIB`,
`LC_LOAD_WEAK_DYLIB`, `LC_REEXPORT_DYLIB` y `LC_LOAD_UPWARD_DYLIB`— mediante
`dp_bundle_inspector::requirements`, que devuelve los install names tal como el
binario los declara, ordenados y sin duplicados entre slices. Nada se resuelve.

- Un nombre que contiene un componente `.framework` no se juzga acá: se juzga una
  sola vez como framework (§3.3), aunque el load command sea de biblioteca.
- Un nombre relativo al loader —prefijo `@rpath/`, `@executable_path/` o
  `@loader_path/`— queda `advisory`: se asume que la biblioteca viaja dentro del
  bundle, y el perfil no la decide.
- Cualquier otro nombre se busca **por ruta exacta** en `[[dylibs]]` y el estado
  del perfil se traduce al del requisito (§5).
- Un nombre que el perfil no registra es `unknown`, nunca un pase.

### 3.3 Frameworks

El nombre sale del componente que termina en `.framework` de una ruta de load
command, o de las entradas `Contents/Frameworks/*.framework` que lista el
inspector. Cada nombre se juzga **una sola vez**, y la copia embebida es la
evidencia que decide: si el bundle trae su propio framework, el requisito es
`advisory` y el perfil no opina, porque el loader lo resuelve desde la aplicación
y la entrada del perfil describe la copia del sistema. Si no está embebido, se
busca por nombre en `[[frameworks]]`; si no hay entrada, es `unknown`.

### 3.4 Versión mínima de macOS

La versión declarada es `LSMinimumSystemVersion` de `Info.plist` y, si esa clave
no está, la que declaran los load commands: `LC_BUILD_VERSION.minos` —sólo con
plataforma macOS— con prioridad sobre `LC_VERSION_MIN_MACOSX.version`, y el mayor
de todos los slices. Después se compara numéricamente con `macos_version` del
perfil; los componentes faltantes valen cero, así que `11` y `11.0` son la misma
exigencia, y por eso `10.9` es más vieja que `10.15` aunque como texto ordene al
revés.

- Declarada ≤ línea base: `satisfied`, con la versión del perfil y su Darwin en el
  detalle.
- Declarada > línea base: `unsatisfied`, porque el Darling fijado no puede ofrecer
  lo que la aplicación pide.
- Sin declaración en el plist ni en los load commands: `unknown` con sujeto
  `none`.
- Un valor que no es una versión numérica con puntos (`Big Sur`): `unknown` que
  nombra el valor leído, nunca un fallo.

La comparación es contra `macos_version`, no contra `darwin_version`; esta última
sólo se cita como referencia.

## 4. La regla de `@rpath`

El perfil compara **por ruta exacta**: la entrada `/usr/lib/libz.1.dylib` coincide
con ese install name y con ningún otro. Por eso `@rpath/libz.1.dylib`,
`@executable_path/libz.1.dylib` y `libz.1.dylib` no coinciden con la entrada
absoluta, y una copia que viaja en el bundle nunca se confunde con la biblioteca
del sistema que el perfil juzgó.

El motor no resuelve `@rpath`: no lee los load commands `LC_RPATH`, no recorre la
lista de rutas de búsqueda, no consulta el prefijo y no toca el sistema de
archivos fuera del bundle. Un nombre relativo al loader se reporta como tal y
queda `advisory`; que efectivamente se cargue del bundle es una suposición del
reporte, no una resolución. Los frameworks siguen la misma regla: un
`@rpath/Plugin.framework/Versions/A/Plugin` cuenta como embebido.

## 5. El perfil de capacidades

El conocimiento vive en `deps/darling-capabilities.toml`, un archivo versionado
que el motor **embebe en el binario** con `include_str!` y valida en cada carga.
El reporte cita así exactamente la versión que se revisa en el repositorio, y
corregir una entrada es un cambio de código revisado, no una propiedad silenciosa
de la máquina. Un perfil inutilizable es un error que nombra el campo y el valor
leído: el motor nunca cae a un valor por omisión.

| Campo | Obligatorio | Qué es |
| --- | --- | --- |
| `profile_version` | Sí | Identifica la revisión de la base de conocimiento; viaja en el reporte |
| `darwin_version` | Sí | Versión de Darwin del pin; se cita como referencia |
| `macos_version` | Sí | macOS que esa familia Darwin representa; es la línea base de comparación |
| `[[frameworks]]` | No | `name` (sin `.framework`), `status` y `note` opcional |
| `[[dylibs]]` | No | `path` (el install name exacto), `status` y `note` opcional |

Estados admitidos, en minúsculas y sin variantes:

| Estado | Requisito resultante | Significado |
| --- | --- | --- |
| `supported` | `satisfied` | Funciona con el Darling fijado |
| `partial` | `advisory` | Funciona con limitaciones documentadas |
| `unsupported` | `unsatisfied` | No funciona con el Darling fijado |
| `unknown` | `unknown` | Nadie lo verificó contra el pin |

La validación es estricta, porque un veredicto construido sobre una base de
conocimiento dudosa es peor que ningún veredicto:

- claves desconocidas se rechazan, también en las entradas: un `statu` mal escrito
  falla en lugar de dejar el campo en un valor por omisión;
- un sujeto repetido en la misma tabla se rechaza, porque el veredicto dependería
  de cuál copia gana;
- `profile_version` vacío, `name` vacío o `path` vacío se rechazan;
- `darwin_version` y `macos_version` deben ser versiones numéricas con puntos;
- un estado fuera de los cuatro nombres se rechaza nombrando la tabla, el sujeto y
  el valor;
- un documento que no es TOML o al que le falta un campo obligatorio se rechaza
  como error de sintaxis.

Actualizar el perfil honestamente:

1. Subir `profile_version` en el mismo cambio que altera el contenido.
2. Escribir `supported`, `partial` o `unsupported` sólo con una prueba registrada
   contra el pin de `deps/darling.toml`, y dejar esa evidencia en `note`. Lo que no
   se verificó se queda en `unknown`.
3. Mantener una entrada por sujeto: nombre de framework sin sufijo, install name
   exacto para las bibliotecas, sin duplicados.
4. Actualizar las dos pruebas de `modules/compatibility-engine/src/profile.rs` que
   fijan el perfil embebido: una afirma que hoy toda entrada es `unknown` y la otra
   enumera los sujetos que este corte debe juzgar.

Hoy todas las entradas son `unknown` —8 frameworks y 8 bibliotecas, con
`profile_version` `1`, Darwin `20.6.0` y macOS `11.6`— porque no hay ninguna
verificación registrada contra el Darling fijado. El veredicto honesto para casi
cualquier aplicación es, por lo tanto, `unknown`, nunca `compatible`.

## 6. Límites

- **El análisis estático es evidencia, no una garantía.** Que el perfil liste una
  biblioteca no dice nada sobre los símbolos, las versiones de ABI ni el
  comportamiento en ejecución; que no la liste no prueba que falte.
- **No hay análisis de símbolos ni de APIs.** No se lee la tabla de símbolos, ni
  los imports, ni ninguna API concreta: una framework `supported` no prueba que
  los símbolos que la aplicación usa existan en el Darling fijado.
- **No hay detección de DRM ni de anticheat.** Una aplicación protegida puede
  obtener `compatible` y no arrancar.
- **No hay análisis dinámico.** Nada se ejecuta, ni la aplicación ni sus
  dependencias, y no hay seguimiento de fallos en tiempo de ejecución.
- **No hay traducción de versión de sistema.** La única comparación de versiones
  es la mínima declarada contra la línea base del perfil; no hay disponibilidad
  por API ni por símbolo.
- **No se leen entitlements ni firmas.** El análisis mira los metadatos del bundle
  y los load commands, nada más.
- **Una biblioteca débil se juzga como requerida.** El inspector no distingue
  `LC_LOAD_WEAK_DYLIB` de `LC_LOAD_DYLIB`, así que una dependencia opcional que el
  perfil marque `unsupported` produce `not_compatible` aunque el loader podría
  seguir sin ella.
- **Sólo se juzga lo que el perfil registra.** Cualquier otro nombre es `unknown`;
  el perfil es la base de conocimiento completa, no una muestra.
- **El plist gana sobre los load commands.** Si `LSMinimumSystemVersion` declara
  una versión más vieja que la de los load commands, se compara la del plist, y una
  exigencia mayor del binario queda tapada.
- **No hay requisitos gráficos, de audio ni de entrada**, ni consulta al motor de
  hardware: el reporte no dice nada sobre GPU, backend ni periféricos.
- **La matriz inicial de aplicaciones de prueba no está en este corte.** Requiere
  bundles reales, que el repositorio no puede versionar; el formato del reporte
  todavía no se contrastó con aplicaciones reales.
- **Un bundle cuyo ejecutable no es Mach-O legible no produce reporte**: el
  inspector lo rechaza y el comando sale con código 1 en lugar de informar un
  requisito desconocido.
