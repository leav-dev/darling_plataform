# ADR-0006: Darling como núcleo externo de compatibilidad

- **Estado:** aceptada
- **Fecha:** 2026-11-04
- **Responsables:** mantenedores de Darling Platform
- **Reemplaza a:** —
- **Reemplazada por:** —

## Contexto

Darling implementa la capa de compatibilidad macOS: carga de binarios Mach-O,
`dyld`, runtime Objective-C, servicios Darwin y traducción de llamadas al kernel.

La plataforma podría absorber partes de Darling, mantener un fork permanente o
integrarlo como dependencia versionada y contribuir upstream.

## Decisión

Darling es una **dependencia externa** de la plataforma:

- el build de Darling sigue siendo CMake y **no** es miembro del workspace de
  Cargo;
- la plataforma integra y administra el entorno; no duplica sus subsistemas;
- cada release fija y documenta la versión o commit de Darling utilizado;
- los parches propios se registran con su motivo y se proponen upstream cuando son
  genéricos;
- un fork se mantiene solo mientras el cambio no esté integrado upstream;
- no se depende de interfaces internas inestables sin una capa adaptadora.

## Alternativas consideradas

| Alternativa | Por qué no |
| --- | --- |
| Fork permanente con cambios propios | Costo de mantenimiento y divergencia creciente |
| Reimplementar partes de Darling (por ejemplo, el cargador) | Duplica el trabajo y contradice el objetivo del proyecto |
| Vendorizar Darling dentro del repositorio | Mezcla ciclos de release y hace imposible seguir upstream |
| Depender directamente de interfaces internas | Se rompe en cada actualización y no hay contrato |

## Consecuencias

### Positivas

- La plataforma se beneficia de las mejoras de Darling sin mantenerlas.
- La versión fijada hace reproducible el diagnóstico y la matriz de pruebas.
- La capa adaptadora permite sobrevivir a cambios internos.

### Negativas y riesgos

- Las interfaces internas de Darling cambian: hay que fijar versiones y probar la
  integración.
- Enviar mejoras upstream lleva tiempo y puede no aceptarse.
- Sin Darling construido, el MVP no puede lanzar aplicaciones: es una dependencia
  pesada y con requisitos de kernel propios.

## Seguimiento

- Documentar la versión o commit fijado en cada release y en el reporte de
  compatibilidad.
- Mantener una lista de parches propios y su estado upstream.
- Verificar los comandos y APIs reales de Darling antes de diseñar el daemon o el
  instalador sobre ellos.
