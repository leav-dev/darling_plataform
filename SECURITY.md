# Seguridad

## Versiones soportadas

El proyecto está en etapa de esqueleto y no publica releases todavía. Solo la rama
`main` recibe correcciones de seguridad. Cuando exista la primera release, esta
sección listará las versiones mantenidas y su ventana de soporte.

## Cómo reportar una vulnerabilidad

**No abras un issue público.** Un reporte público convierte la vulnerabilidad en
exploitable antes de que exista un parche.

> **Contacto pendiente de definir.** Antes de la primera release hay que
> habilitar un canal privado (por ejemplo, un buzón de seguridad o los avisos
> privados de seguridad del alojamiento del repositorio) y reemplazar este
> párrafo. Hasta entonces, no hay un canal privado publicado.

Un reporte útil incluye:

- versión o commit afectado, distribución, kernel y arquitectura;
- qué componente está involucrado (CLI, DHI, prefijo, instalador, catálogo,
  daemon);
- pasos mínimos para reproducir, con archivos de prueba si son necesarios;
- impacto observado: lectura o escritura fuera del destino permitido, ejecución
  no autorizada, elevación a root, exposición de datos;
- si hace falta, una prueba de concepto mínima.

No hay un SLA definido todavía. Se acusa recibo, se evalúa el impacto y se
coordina la divulgación con quien reporta antes de publicar detalles.

## Modelo de amenaza

La plataforma corre **sin root** y trata como hostiles todas las entradas que
provienen de afuera:

| Entrada | Riesgo principal |
| --- | --- |
| Bundles `.app`, ZIP, DMG, PKG | Path traversal, symlinks maliciosos, escritura fuera del destino, scripts de instalación |
| `Info.plist` | Metadatos falsos, rutas que escapan del bundle, ejecutable declarado inexistente |
| Cabeceras Mach-O | Arquitecturas o dependencias falsas que alteran el análisis |
| Iconos y recursos | Parseo de formatos no confiables |
| IPC del daemon | Solicitudes manipuladas para lanzar procesos o tocar archivos arbitrarios |

Controles esperados:

- Extracción en staging y validación de rutas antes de copiar nada.
- Consentimiento explícito antes de ejecutar instaladores con scripts.
- Operaciones tipadas y validadas en el daemon; nunca shell arbitrario.
- Un único DPREFIX dedicado, con locks que serializan las operaciones que lo
  modifican.
- Sin elevación de privilegios: el escaneo de hardware y la administración
  ordinaria no las necesitan.
- Logs estructurados sin secretos ni contenido privado de las aplicaciones.

## Fuera de alcance

- Vulnerabilidades en Darling, en drivers gráficos, en el kernel o en la
  biblioteca C del sistema: se reportan a esos proyectos.
- Compatibilidad con DRM, anticheat o servicios de terceros: la plataforma
  documenta límites, no los elude.
- Modificación automática del sistema anfitrión: no es una función, es un
  no-objetivo.

## Buenas prácticas para quien contribuye

- Corré `scripts/ci/check.sh` antes de cada PR; no desactives lints para "destrabar"
  un cambio.
- Si agregás una operación que modifica archivos o procesos, agregá también la
  prueba que demuestra que rechaza las entradas peligrosas.
- Si agregás una dependencia, revisá su licencia y su historial de seguridad.
