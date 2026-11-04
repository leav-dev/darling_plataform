# ADR-0002: Un DPREFIX compartido y dedicado

- **Estado:** aceptada
- **Fecha:** 2026-11-04
- **Responsables:** mantenedores de Darling Platform
- **Reemplaza a:** —
- **Reemplazada por:** —

## Contexto

Darling organiza su entorno en un *prefix* cuyo ciclo de vida administra. La
plataforma tiene que decidir si cada aplicación recibe su propio prefijo o si todas
comparten uno.

Un prefijo por aplicación daría aislamiento, pero multiplica el costo: cada
prefijo implica inicialización, espacio en disco, actualizaciones y servicios
propios, y las aplicaciones dejan de compartir bibliotecas y estado como lo harían
en un macOS real.

## Decisión

Todas las aplicaciones administradas usan **un único DPREFIX dedicado y
compartido**, distinto del prefijo convencional del usuario si existe:

- ubicación: `~/.local/share/darling-platform/prefix/`;
- la plataforma configura `DPREFIX` explícitamente en cada invocación;
- Darling administra la estructura interna del prefijo; la plataforma no fabrica
  directorios que correspondan a Darling;
- catálogo, iconos, logs y configuración viven **fuera** del prefijo;
- el Shared Prefix Manager es la única autoridad sobre su ciclo de vida.

## Alternativas consideradas

| Alternativa | Por qué no |
| --- | --- |
| Un prefijo por aplicación | Costo de inicialización, espacio y actualización; contradice cómo funcionan las aplicaciones en macOS |
| Usar el prefijo convencional del usuario | Interfiere con una instalación existente y con su estado |
| Prefijo efímero por sesión | Pierde instalaciones y estado; inviable para un catálogo persistente |
| Aislamiento por contenedor o namespace | Fuera del MVP; requiere una decisión de arquitectura propia |

## Consecuencias

### Positivas

- Inicialización y actualización del entorno se pagan una vez.
- Las aplicaciones comparten bibliotecas y servicios, como en macOS.
- Una sola ubicación para backup, diagnóstico y validación.

### Negativas y riesgos

- **Estado compartido**: una instalación o actualización puede afectar a otras
  aplicaciones. Requiere locks de escritura y operaciones transaccionales.
- **Apagado coordinado**: no se puede detener el entorno mientras haya procesos
  gestionados activos.
- **Sin aislamiento por aplicación**: no forma parte del MVP; incorporarlo exige
  un ADR nuevo y un rediseño del modelo de datos.
- **Backup delicado**: mover o copiar el prefijo con Darling activo no es seguro y
  debe documentarse.

## Seguimiento

- Definir y probar la política de locks antes de implementar el instalador.
- Documentar el procedimiento de backup y recuperación junto con el Shared Prefix
  Manager.
- Si aparece un caso que exija aislamiento, escribir el ADR que reemplace a este
  antes de implementarlo.
