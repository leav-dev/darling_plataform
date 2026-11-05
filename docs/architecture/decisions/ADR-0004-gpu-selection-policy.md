# ADR-0004: Política Dedicated GPU First

- **Estado:** aceptada
- **Fecha:** 2026-11-04
- **Responsables:** mantenedores de Darling Platform
- **Reemplaza a:** —
- **Reemplazada por:** —

## Contexto

En equipos híbridos convive una GPU dedicada con una integrada. Elegir mal
degrada el rendimiento de forma visible y puede romper la
presentación en pantalla, porque la integrada suele estar conectada al display
mientras la dedicada hace el trabajo pesado.

El escaneo de hardware y la decisión de ejecución tienen tiempos de vida
distintos: el escaneo describe el equipo; la decisión depende de la aplicación, del
perfil y de las preferencias del usuario.

## Decisión

Cuando el sistema tenga GPU dedicada e integrada:

1. detectar y conservar ambas;
2. priorizar la **dedicada** para renderizado y ejecución de aplicaciones;
3. mantener la integrada disponible para pantallas, presentación y sistemas
   híbridos;
4. evaluar la integrada como alternativa solo si la dedicada no está disponible o
   no cumple los requisitos;
5. permitir selección manual cuando sea técnicamente posible.

La política se aplica en el **runtime**. El motor de hardware describe los
dispositivos y no los oculta ni los descarta.

## Alternativas consideradas

| Alternativa | Por qué no |
| --- | --- |
| Dejar que el driver o el compositor decida | No es predecible ni diagnosticable desde la plataforma |
| Elegir siempre la integrada | Contradice el objetivo de rendimiento en aplicaciones |
| Elegir la primera GPU enumerada | El orden de enumeración no expresa capacidad ni rol |
| Ocultar la integrada del reporte | Impide diagnóstico y rompe casos híbridos |

## Consecuencias

### Positivas

- Comportamiento predecible en equipos híbridos.
- El reporte de hardware conserva toda la evidencia para diagnosticar.
- La selección manual queda disponible sin reimplementar la detección.

### Negativas y riesgos

- Requiere conocer bien qué significa "dedicada" en cada driver y vendor; la
  heurística puede fallar en equipos inusuales.
- Una GPU dedicada sin soporte funcional de Vulkan puede ser peor opción que la
  integrada: la política necesita capacidades verificadas, no solo el tipo de
  dispositivo.
- Sin equipos híbridos reales en la matriz de pruebas, la política no está
  validada.

## Seguimiento

- Definir el criterio de "dedicada" a partir de datos de PCI y de Vulkan, no del
  nombre del dispositivo.
- Incorporar equipos híbridos (AMD, NVIDIA e Intel) a la matriz de pruebas.
- Registrar en el reporte de compatibilidad qué GPU se eligió y por qué.
