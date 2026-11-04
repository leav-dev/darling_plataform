# ADR-0005: DHI separado de las decisiones de ejecución

- **Estado:** aceptada
- **Fecha:** 2026-11-04
- **Responsables:** mantenedores de Darling Platform
- **Reemplaza a:** —
- **Reemplazada por:** —

## Contexto

El Hardware Intelligence Engine descubre hardware y capacidades. Sería tentador
que esa misma pieza decidiera cómo lanzar una aplicación: tiene los datos, conoce
las GPUs y sabe qué backends existen.

Pero los datos y la política cambian por motivos distintos. Un escaneo sirve para
diagnóstico, para el reporte de compatibilidad y para el catálogo, aunque no se
lance nada. Y la elección de dispositivos depende de la aplicación, del perfil de
runtime y de las preferencias del usuario, no del hardware por sí solo.

## Decisión

El DHI **describe**; el runtime **decide**.

- El DHI produce un `HardwareReport` versionado con CPU, memoria, sistema, GPUs
  PCI, capacidades Vulkan, pantallas y diagnósticos.
- El DHI no selecciona GPU ni backend, no instala drivers y no modifica el host.
- El runtime combina el perfil del DHI con las políticas del usuario y del perfil
  de ejecución para elegir estrategia y dispositivos.
- La detección es parcial y tolerante a fallos: un subsistema ausente degrada el
  reporte y agrega un diagnóstico; nunca invalida el resto.

## Alternativas consideradas

| Alternativa | Por qué no |
| --- | --- |
| Que el DHI elija GPU y backend | Mezcla detección con política; imposible de probar por separado |
| Detección dentro del daemon | Acopla el escaneo al ciclo de vida de un proceso residente y a D-Bus |
| Reporte sin versionado | Rompe consumidores cada vez que cambia un campo |

## Consecuencias

### Positivas

- El reporte se puede generar, probar y consumir sin lanzar nada.
- La política Dedicated GPU First se puede cambiar sin tocar la detección.
- Los fixtures de hardware permiten probar análisis sin tener todos los equipos.

### Negativas y riesgos

- Dos lugares que evolucionan: el formato del reporte y las reglas del runtime;
  ambos necesitan versionado.
- Riesgo de que el runtime ignore información que el DHI sí detectó; la
  trazabilidad de la decisión debe registrarse.

## Seguimiento

- Versionar el esquema del reporte desde la primera implementación.
- Registrar en el diagnóstico qué evidencia del reporte consumió el runtime para
  elegir.
