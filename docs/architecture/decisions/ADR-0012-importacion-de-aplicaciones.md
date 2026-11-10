# ADR-0012: Importación de aplicaciones con staging, `.app`/`.zip` y catálogo SQLite

- **Estado:** aceptada
- **Fecha:** 2026-11-09
- **Responsables:** mantenedores de Darling Platform
- **Reemplaza a:** —
- **Reemplazada por:** —

## Contexto

La Fase 3 del plan pide importar una aplicación macOS sin ejecutarla:
inspeccionar el bundle, extraerlo en staging, validarlo, instalarlo en el DPREFIX
compartido y dejarlo en un catálogo consultable, de forma repetible y sin
instalaciones parciales silenciosas.

El plan enumera cinco formatos por prioridad (`.app`, `.zip`, `.dmg`, `.pkg`,
`.xip`) y pide, a la vez, rollback donde sea posible, conversión de iconos `.icns`
y advertencias para PKG porque sus scripts pueden impedir un rollback completo.
También fija el almacenamiento del catálogo: SQLite, fuera del DPREFIX, con
migraciones versionadas y una tabla `applications` orientativa.

Hacer todo eso de una vez mezcla tres problemas de naturaleza distinta: un
formato ya extraído, un comprimido y un instalador que ejecuta código de terceros.
El repo, además, tiene convenciones fijas: CLI sin `clap`, `Response { code,
stdout, stderr }`, exit 2 para uso y 1 para fallo, errores hand-rolled, pruebas
inline con fixtures y sin root, y ninguna dependencia de sistema nueva en el
`.deb` (ADR-0010).

## Decisión

1. **Este corte importa solo `.app` y `.zip`, en ese orden.** Un `.app` es un
   bundle ya extraído y un `.zip` es un comprimido que lo contiene; los dos caben
   en un flujo de copia y validación. El formato se identifica por extensión, sin
   distinguir mayúsculas, y el contenido no se inspecciona para adivinar otro
   formato. `.dmg`, `.pkg` y `.xip` se rechazan con una lista de los formatos
   soportados: cada uno necesita montaje o un instalador propio, y `.pkg` ejecuta
   scripts con efectos externos. Dependencias: `plist` (XML y binario), `zip`
   (solo *stored* y *deflate*, en Rust puro) y `rusqlite` con `bundled`.
2. **El staging vive dentro del sistema de archivos del destino y la instalación
   es un solo `rename`.** El bundle se prepara en
   `<applications_dir>/.staging/<installation_id>` y se coloca con
   `std::fs::rename`. Así el destino nunca contiene media aplicación, y las dos
   operaciones que pueden fallar a mitad (copiar y extraer) ocurren lejos de la
   ruta final. La guarda del staging borra su directorio en todos los caminos de
   salida.
3. **El catálogo es SQLite, fuera del DPREFIX, con migraciones en
   `user_version`.** Un archivo
   `<raíz de datos>/darling-platform/library.sqlite3`. La versión viaja dentro del
   archivo, cada paso corre en una transacción, solo se aplican los pasos que
   faltan y un catálogo escrito por un build más nuevo se rechaza en lugar de
   leerse con un esquema desconocido. El CLI deriva la ruta de la misma ubicación
   que resuelve el prefijo, para que las dos no puedan discrepar.
4. **`application_id` es el bundle identifier.** Como además el id es la clave
   primaria de `applications`, hay una fila por bundle id: reinstalar la misma
   aplicación actualiza su fila en lugar de agregar una segunda. La restricción
   única `(bundle_id, bundle_path)` sigue existiendo y permite dos rutas del mismo
   bundle id a nivel de esquema, pero el instalador no la usa para convivir con
   dos versiones: el destino lo deriva del nombre del bundle, así que una segunda
   versión en otra ruta tiene que pedir `--replace` o cambiar de nombre.
5. **Reemplazar es opt-in explícito, e intercambia y actualiza.** Sin `--replace`,
   un destino ocupado es un error que nombra la política. Con `--replace`, la
   copia instalada se mueve a `.staging/<installation_id>-replaced`, el bundle
   preparado se renombra al destino y, si ese segundo `rename` falla, la copia
   movida se restituye. La fila se actualiza con un *upsert* que sobrescribe todas
   las columnas mutables, y las dos instalaciones quedan como dos filas de
   historial.
6. **Los intentos fallidos quedan en el historial.** Todo error posterior a la
   aceptación del formato escribe una fila de `installations` con
   `result = failed`, sin `application_id` y con el mensaje del error en `detail`.
   Un origen con formato no soportado no es un intento y no deja fila, porque
   `installations.format` guarda el vocabulario de formatos. Aplicación e intento
   se escriben en una sola transacción: si el catálogo rechaza algo, el sistema de
   archivos vuelve atrás.
7. **La entrada no es confiable y se valida antes de escribir.** Las rutas
   declaradas en `Info.plist` se validan léxicamente (nada absoluto, ningún `..`)
   y físicamente (ningún symlink que salga del bundle), y el ejecutable declarado
   debe existir, ser archivo regular y tener cabecera Mach-O. Del ZIP se rechazan
   las entradas absolutas, las que llevan `..`, las que tienen un byte NUL, las
   que el lector no puede encerrar en el staging y los enlaces simbólicos, con
   topes de 200 000 entradas y 8 GiB descomprimidos comprobados contra lo
   declarado y contra los bytes copiados. En un `.app` los enlaces se recrean con
   su mismo texto, pero solo después de probar que su destino canónico sigue
   dentro del bundle de origen; en un host que no puede representar enlaces la
   copia se rechaza. El nombre del destino se sanea hasta ser un único componente
   de ruta.
8. **El rollback de una instalación completada se difiere.** Este corte deshace
   lo que falla *dentro* de una misma instalación (copia nueva borrada, copia
   reemplazada restituida), pero una vez que el catálogo acepta no queda de dónde
   volver: la fila está actualizada y la copia anterior se borra. La conversión de
   iconos `.icns` también se difiere, porque todavía no hay interfaz que los
   consuma; el catálogo guarda la ruta del icono dentro del bundle instalado.

## Alternativas consideradas

| Alternativa | Por qué no |
| --- | --- |
| Extraer directamente en el destino final | Un fallo a mitad deja media aplicación en la ruta que Darling lee, que es exactamente la instalación parcial silenciosa que el criterio de salida prohíbe. El staging más un `rename` cuesta un directorio temporal y elimina el estado intermedio visible |
| Catálogo en JSON en lugar de SQLite | El plan fija SQLite y las migraciones versionadas. Un JSON obliga a reescribir el archivo entero, no da transacción entre la fila de la aplicación y la del intento, y no puede hacer cumplir la unicidad de `(bundle_id, bundle_path)` |
| Incluir `.dmg` y `.pkg` en este corte | El DMG requiere montar y desmontar, y el PKG ejecuta scripts que pueden tocar el host fuera del prefijo. Meterlos antes de verificar el flujo de `.app`/`.zip` habría mezclado dos riesgos distintos en un mismo corte |
| Sobrescribir el destino en silencio | El plan prohíbe sobrescribir sin política explícita, y una reinstalación silenciosa destruye una instalación que el operador quizá quería conservar. `--replace` deja la decisión escrita en el comando |
| Una fila de `applications` por instalación, no por bundle | Duplica la misma aplicación en el catálogo en cada reinstalación y deja varias filas que el usuario no distingue. El historial ya guarda cada intento; la fila de aplicación describe el estado actual |
| Instalar en staging y copiar al destino al final | Copiar deja una ventana en la que el destino existe incompleto. En el mismo sistema de archivos un `rename` no tiene esa ventana |
| Dejar el historial solo con intentos exitosos | Un fallo sin rastro no se puede diagnosticar después, y el plan pide registrar el resultado de la instalación, no solo el éxito |

## Consecuencias

### Positivas

- El criterio de salida de la Fase 3 se cumple para `.app` y `.zip`: importar,
  inspeccionar, instalar y registrar es repetible, y un fallo no deja instalación
  parcial.
- El destino pasa de inexistente a completo con una sola operación del sistema de
  archivos, así que ningún consumidor ve un bundle a medio copiar.
- El catálogo cabe en un archivo, se copia con su versión y no depende de ningún
  servicio: `bundled` evita sumar un `Depends` al `.deb`.
- Cada intento, exitoso o no, queda con origen, formato, resultado, motivo y
  fecha, que es la evidencia que hace diagnosticable un fallo de importación.
- Las reglas de seguridad se prueban con archivos malformados y fixtures, sin
  root y sin ejecutar nada.

### Negativas y riesgos

- **No hay rollback de una instalación completada.** Reemplazar borra la copia
  anterior cuando el catálogo acepta; volver atrás exige reinstalar el origen
  anterior. Un `--replace` equivocado no se deshace con el catálogo.
- **Un bundle id ocupa una sola fila.** Dos versiones de la misma aplicación no
  conviven: la segunda instalación sin `--replace` choca contra el catálogo
  aunque su destino esté libre, y con `--replace` reemplaza la fila de la
  primera.
- **El límite de 8 GiB descomprimidos y el de 200 000 entradas son arbitrarios
  pero necesarios.** Un bundle legítimo por encima del tope se rechaza y hay que
  instalarlo como `.app` extraído.
- **La extracción del ZIP no reemplaza la validación del bundle.** Un archivo
  puede ser un ZIP perfectamente válido que no contenga ninguna aplicación: se
  detecta recién al buscar el `.app` preparado.
- **Las reglas de enlaces simbólicos del `.app` solo están cubiertas por pruebas
  `#[cfg(unix)]`.** La rama que rechaza la copia en un host sin enlaces existe
  pero no tiene prueba propia en este corte.
- **El catálogo no es la verdad sobre el disco.** Una fila puede apuntar a un
  bundle que alguien borró; verificarlo antes de lanzar o desinstalar queda para
  el consumidor.
- **`installation_id` es único por intento.** Reusarlo hace que un fallo no se
  pueda registrar, y el error combina el fallo original con esa imposibilidad;
  un llamador que no genere ids únicos pierde historial.
- **La escritura de archivos y la extracción de comprimidos de entrada no
  confiable son riesgo alto del corte.** Mitigado por validación previa a la
  escritura, topes y pruebas con archivos malformados, no por revisión de cada
  byte extraído; de ahí que la verificación incluya revisión independiente.

## Seguimiento

- Diseñar e implementar rollback donde sea posible: qué se guarda de la copia
  reemplazada, por cuánto tiempo y cómo se ofrece deshacer.
- `.dmg` en su propio corte, después de verificar el flujo de `.app`/`.zip`:
  montar, copiar el bundle, desmontar y limpiar el punto de montaje.
- `.pkg` como flujo explícito y de mayor riesgo: advertencia de que los scripts
  pueden impedir un rollback completo y consentimiento antes de ejecutarlos.
- `.xip` después de investigar soporte y herramientas adecuadas.
- Conversión de `.icns` a un formato usable por la interfaz, conservando el
  original, cuando exista el Application Center que los consuma.
- Consulta de historial desde el CLI (hoy el catálogo lo guarda pero `dp app list`
  no lo imprime) y la tabla de instancias en ejecución cuando algo las lance.
