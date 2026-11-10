# ADR-0013: Motor de compatibilidad estático con perfil de capacidades versionado

- **Estado:** aceptada
- **Fecha:** 2026-11-09
- **Responsables:** mantenedores de Darling Platform
- **Reemplaza a:** —
- **Reemplazada por:** —

## Contexto

La Fase 4 del plan (`PLAN_DARLING_PLATFORM.md` §5.7) pide analizar, estática y
dinámicamente cuando sea posible, los requisitos de una aplicación macOS y
compararlos con las capacidades conocidas de Darling y del host. Enumera el
análisis estático inicial —arquitectura y slices Mach-O, tipo de binario y
ejecutable principal, dependencias dinámicas, frameworks referenciados, versión
mínima de macOS, metadatos del bundle y recursos identificables— y exige que el
motor produzca resultado general, evidencias, requisitos satisfechos y no
satisfechos, elementos desconocidos, advertencias, y la versión del analizador y
del perfil usados.

El mismo apartado fija la advertencia que ordena todo el diseño: «No convertir un
análisis estático favorable en una garantía de ejecución. La compatibilidad real
requiere pruebas». El criterio de salida de la fase (§10) es que el sistema
diferencie con claridad compatible, parcialmente compatible, no compatible y
desconocido, con razones y evidencias, y sus entregables incluyen una matriz
inicial de aplicaciones de prueba.

Tres hechos del repo acotan lo que se puede construir sin inventar:

1. El pin de Darling existe y está verificado como instalación (ADR-0009), pero
   nadie registró todavía qué frameworks y bibliotecas funcionan sobre él. Un
   motor que afirme compatibilidad hoy estaría inventando evidencia.
2. El repositorio no puede versionar aplicaciones macOS reales: la matriz inicial
   del plan necesita bundles que este repo no puede distribuir.
3. Los datos de entrada son archivos de terceros —`Info.plist` y cabeceras Mach-O—
   y ya hay un componente que los valida para no confiar en ellos: el Bundle
   Inspector, que el instalador usa desde la Fase 3.

Las convenciones fijas del proyecto también cuentan: CLI sin `clap`,
`Response { code, stdout, stderr }`, código 1 para fallo de ejecución y 2 para uso
incorrecto, errores hand-rolled, pruebas inline con fixtures, sin root y sin
dependencias de sistema nuevas (ADR-0010).

## Decisión

1. **Este corte analiza sólo estáticamente.** Se leen los metadatos del bundle y
   los load commands del ejecutable; nada se ejecuta, ni la aplicación ni sus
   dependencias, y no hay seguimiento de fallos en ejecución. El análisis dinámico
   queda para un corte propio, cuando exista algo que lanzar de forma verificada.
2. **El perfil de capacidades es un TOML versionado embebido en el binario.** Vive
   en `deps/darling-capabilities.toml` y se incluye con `include_str!`, así que el
   reporte cita exactamente la `profile_version` que se revisa en el repositorio y
   corregir una entrada es un cambio de código revisado, no una propiedad
   silenciosa de la máquina donde corre el comando. Se valida en cada carga y un
   perfil inutilizable es un error que nombra el campo y el valor: el motor nunca
   cae a un valor por omisión.
3. **El perfil arranca en `unknown`.** `supported`, `partial` y `unsupported` son
   afirmaciones que exigen una prueba registrada contra el pin de
   `deps/darling.toml`; lo que no se verificó se escribe `unknown`, y una entrada
   `unknown` produce un requisito desconocido. El perfil inicial registra los
   sujetos que este corte debe juzgar —ocho frameworks y ocho bibliotecas del
   sistema— todos en `unknown`, con una nota que dice por qué.
4. **El veredicto es el del peor requisito.** Cualquier requisito insatisfecho da
   `not_compatible`; si no hay ninguno, cualquier desconocido da `unknown`; si no,
   cualquier advertencia da `partial`; sólo una lista no vacía con todo satisfecho
   da `compatible`. Una lista vacía es `unknown`: si no se comprobó nada, no se
   sabe nada. Un `unknown` nunca se presenta como compatible ni como fallo.
5. **Cada requisito lleva su evidencia.** El reporte es
   `schema_version`, `verdict`, `analyser_version`, `profile_version` y una lista
   de `requirements[]` con `kind`, `subject`, `status` y `detail`, donde el detalle
   nombra la ruta, la versión y la nota del perfil en las que se apoya el
   resultado. Enums en `snake_case`, ausencias omitidas en lugar de `null` y
   colecciones siempre presentes, como el resto de los reportes de la plataforma.
6. **El analizador y el perfil viajan en el reporte.** `schema_version` describe la
   forma JSON (hoy `1.0`), `analyser_version` describe las reglas de análisis (hoy
   `1`) y `profile_version` viene del perfil. Cambiar el significado de un
   veredicto sube el analizador; agregar campos mantiene el esquema.
7. **El motor no reimplementa el parseo Mach-O.** `dp-bundle-inspector` expone
   `requirements()`, que devuelve arquitecturas, install names de los cuatro load
   commands de dependencia y la versión mínima declarada, con los mismos límites y
   validaciones con los que ya inspecciona el bundle. El motor sólo interpreta esa
   evidencia; así hay una sola verdad sobre el parseo de entrada no confiable y un
   solo lugar que corregir.
8. **Nada se resuelve y nada se escribe.** El motor no resuelve install names, no
   lee `LC_RPATH`, no busca en el prefijo, no abre el catálogo y no toma el lock.
   La arquitectura del host la aporta el llamador —el CLI pasa
   `std::env::consts::ARCH`—, así que el mismo bundle y el mismo perfil producen
   siempre el mismo reporte y las pruebas no dependen de la máquina.
9. **El análisis no condiciona la instalación en este corte.** `dp app install` no
   consulta el motor y ningún veredicto bloquea nada: un perfil incompleto no puede
   impedir una instalación que antes funcionaba. El gating es una decisión de un
   corte posterior, con su propio contrato.

## Alternativas consideradas

| Alternativa | Por qué no |
| --- | --- |
| Tabla de capacidades fija en el código | Un `match` sobre nombres dispersa la base de conocimiento entre módulos, no se puede versionar como una unidad ni citar en el reporte, y corregir una entrada deja de ser un cambio legible. El TOML embebido da versión, validación y una revisión que se puede leer completa |
| Sondear el Darling fijado en el momento del análisis | Habría que ejecutar algo dentro del prefijo para preguntar qué funciona: eso contradice el «no ejecutar» de este corte, hace el análisis dependiente del host y del estado del prefijo, y convierte una consulta de sólo lectura en una operación con efectos. Además, sin pruebas registradas, el sondeo sólo devolvería la ausencia de evidencia con más pasos |
| Tratar los requisitos desconocidos como fallos | Un `unknown` convertido en `not_compatible` afirma una incompatibilidad que nadie comprobó; con un perfil inicial todo en `unknown`, toda aplicación sería «no compatible» y el reporte mentiría en la dirección más costosa. El veredicto propio `unknown` es justamente el que el criterio de salida de la fase pide |
| Gatear `dp app install` desde ahora | Bloquear instalaciones con una base de conocimiento que no verificó nada rompe un flujo que hoy funciona y no aporta información: el mismo reporte se obtiene con `dp app compat` sin bloquear. El gating necesita primero un perfil con evidencia y una política de qué hacer con `unknown` |
| Reimplementar el parseo Mach-O dentro del motor | Duplicaría un parser de entrada no confiable que ya existe, con sus límites de tamaño, su orden de bytes y sus pruebas. Dos implementaciones del mismo formato divergen, y el motor necesita muy poco del archivo: arquitecturas, cuatro load commands y dos de versión. Depender del inspector concentra el riesgo en un componente y deja el motor testeable con fixtures mínimos |
| Detectar el host dentro del motor | Leer `std::env::consts::ARCH` en el módulo haría el reporte dependiente de la máquina que corre el comando y obligaría a simular el host en cada prueba. Inyectar la arquitectura deja el motor determinista y permite analizar un bundle para otra máquina |
| Escribir el veredicto en el catálogo de aplicaciones | El catálogo describe lo instalado, no lo que se pensó de un bundle en un momento; un veredicto guardado sin la `profile_version` que lo produjo envejece en silencio. La fila del catálogo sigue sin análisis de compatibilidad hasta que exista un consumidor que lo pida |

## Consecuencias

### Positivas

- El criterio de salida de la Fase 4 se cumple para el análisis estático: los
  cuatro resultados se distinguen con razones y evidencias, y el reporte lleva las
  versiones que lo produjeron.
- El motor dice la verdad sobre lo que sabe: con el perfil en `unknown`, el
  veredicto es `unknown`, no un falso `compatible` ni un falso fallo.
- Corregir el perfil es un cambio revisado y versionado: el reporte cita la
  revisión del repositorio, no el estado de la máquina.
- El análisis es determinista y barato de probar: la arquitectura del host llega
  como dato y toda la evidencia se arma con fixtures Mach-O pequeños.
- No hay riesgo nuevo para el prefijo ni para el catálogo: el comando no los toca,
  no escribe nada y no puede dejar estado intermedio.
- Hay una sola implementación del parseo Mach-O de entrada no confiable.

### Negativas y riesgos

- **El veredicto se puede leer como garantía.** Es el riesgo declarado de este
  corte: un `partial` o incluso un `compatible` no dice nada sobre símbolos ni
  comportamiento. Se mitiga repitiendo el límite en el contrato del crate, en la
  documentación y en el ADR, no con una prueba.
- **El perfil es un juicio humano.** Las 16 entradas están en `unknown` porque no
  hay verificación registrada: hasta que exista la matriz, el motor dirá `unknown`
  a casi toda aplicación y su utilidad práctica será acotada. Un perfil que se
  llene sin evidencia sería peor, porque produciría veredictos con apariencia de
  certeza.
- **Nada ata el perfil al pin automáticamente.** `darwin_version` se cita como
  referencia y no se compara con `deps/darling.toml`; subir el pin exige revisar el
  perfil a mano, y un perfil olvidado describiría un Darling que ya no es el
  fijado.
- **La comparación por ruta exacta produce desconocidos legítimos.** Una
  biblioteca del sistema declarada con otro nombre o con otro sufijo de versión no
  coincide con la entrada del perfil y queda `unknown`; el reporte no la marca como
  fallo, pero tampoco la reconoce.
- **Las dependencias débiles se juzgan como requeridas.** El inspector no distingue
  `LC_LOAD_WEAK_DYLIB` de `LC_LOAD_DYLIB`, así que una biblioteca opcional que el
  perfil marque `unsupported` puede producir `not_compatible` donde dyld seguiría
  adelante.
- **Una referencia relativa al loader se asume dentro del bundle.** No se resuelve
  `@rpath`: si esa ruta terminara resolviendo a una copia del prefijo, el reporte
  seguiría diciendo `advisory` por haberla tratado como parte del bundle.
- **El plist tapa lo que declaran los load commands.** Si `LSMinimumSystemVersion`
  es más vieja que el `minos` del binario, se compara la del plist y una exigencia
  mayor queda invisible en el reporte.
- **El formato del reporte no se contrastó con aplicaciones reales.** La matriz
  inicial no entra en este corte porque el repo no puede versionar esos bundles, así
  que los campos y los detalles se fijaron con fixtures.
- **Se leen cabeceras Mach-O de entrada no confiable.** Mitigado por el parseo
  acotado del inspector —recorrido limitado por `sizeofcmds`, longitudes validadas,
  sólo seis load commands interpretados— y por pruebas con archivos malformados, no
  por revisión de cada byte.

## Seguimiento

- Matriz inicial de aplicaciones de prueba: qué bundles (reales o generados de
  forma reproducible) ejercitan los cuatro veredictos y cómo se registra su
  resultado junto a la `profile_version` que lo produjo.
- Primera verificación real contra el pin: qué frameworks y bibliotecas pasan de
  `unknown` a `supported`, `partial` o `unsupported`, con la evidencia anotada en el
  perfil y la `profile_version` subida.
- Revisar el perfil cada vez que cambie el pin de Darling, y decidir si la
  correspondencia entre `darwin_version` y el pin se puede comprobar en una prueba.
- Análisis de símbolos y APIs importadas, que es lo que hoy separa «la biblioteca
  existe» de «los símbolos que la aplicación usa existen».
- Análisis dinámico acotado, con registro de fallos al arrancar, cuando exista una
  forma verificada de lanzar la aplicación.
- Detección de DRM y anticheat, que ningún análisis estático puede sustituir.
- Requisitos gráficos, de audio y de entrada, correlacionados con el reporte de
  hardware del DHI.
- Gating consultivo en `dp app install`: advertir sin bloquear, con una política
  explícita para `unknown`, recién cuando el perfil tenga evidencia.
- Resolver `@rpath` y las rutas de búsqueda del loader para distinguir una copia
  embebida de una biblioteca del prefijo.
