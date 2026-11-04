# ADR-0001: Monorepo como estrategia inicial

- **Estado:** aceptada
- **Fecha:** 2026-11-04
- **Responsables:** mantenedores de Darling Platform
- **Reemplaza a:** —
- **Reemplazada por:** —

## Contexto

La plataforma nace con varios módulos que comparten contratos: modelos de dominio,
esquemas IPC, estados del prefijo, formato del reporte de hardware y del reporte de
compatibilidad. Mientras la arquitectura evoluciona, esos contratos cambian juntos
y con frecuencia.

Todavía no hay usuarios externos ni necesidad de publicar crates por separado. Lo
que sí hace falta es que un cambio de contrato y sus consumidores se puedan revisar,
probar e integrar en el mismo paso.

## Decisión

Un único repositorio con:

- workspace de Cargo para los crates Rust;
- un `CMakeLists.txt` raíz que coordina el build y la instalación;
- documentación de arquitectura, ADR y documentos de trabajo en el mismo árbol;
- CI único que verifica todo el workspace.

Los módulos se separan por **crates**, no por repositorios. Se extrae un
repositorio propio solo cuando exista una razón concreta: release independiente,
otro ciclo de vida o consumidores externos reales.

## Alternativas consideradas

| Alternativa | Por qué no ahora |
| --- | --- |
| Multi-repositorio desde el inicio | Duplica CI y obliga a versionar contratos entre repos antes de que existan |
| Submódulos git | Añade fricción de sincronización sin resolver los contratos compartidos |
| Un solo crate | Impide que las fronteras entre módulos sean verificables por el compilador |
| Monorepo con carpetas sin crates | Pierde la verificación de dependencias que da Cargo |

## Consecuencias

### Positivas

- Un cambio de contrato y sus consumidores se revisan en un único diff.
- `cargo test --workspace` y un solo pipeline cubren todo el código.
- Las fronteras entre módulos son reales: un crate no puede usar lo que no declara.
- Refactors grandes son baratos mientras la arquitectura se estabiliza.

### Negativas y riesgos

- El repositorio crece y con él el tiempo de CI y el costo de revisión.
- Sin disciplina, las fronteras se diluyen en dependencias cruzadas.
- Un módulo con ciclo de release propio va a chocar con esta decisión.

## Seguimiento

- Revisar si algún crate necesita release independiente o consumidores externos.
- Si el tiempo de CI deja de ser aceptable, segmentar jobs antes de partir el
  repositorio.
- Vigilar dependencias cruzadas entre módulos en cada revisión.
