# ADR-0008: Ciclo de vida verificado del DPREFIX de Darling

- **Estado:** aceptada
- **Fecha:** 2026-11-04
- **Responsables:** mantenedores de Darling Platform
- **Reemplaza a:** —
- **Reemplazada por:** —

## Contexto

La Fase 2 del plan pide "implementar inicialización, validación, inicio y apagado"
del DPREFIX. Para diseñarlo hace falta saber qué ofrece Darling de verdad. La regla
del plan es explícita: no asumir comandos ni comportamientos sin comprobarlos.

Se auditó la documentación publicada y el código de arranque de Darling
(`src/startup/darling.c`). El resultado no coincide con la intuición de una API de
ciclo de vida tipo servicio:

| ID | Hecho verificado | Fuente |
| --- | --- | --- |
| F1 | El prefijo se selecciona con `DPREFIX`; por defecto `~/.darling`. Debe estar **exportado** en el shell: usarlo solo al invocar Darling "no funciona como se espera". | `docs.darlinghq.org/darling-prefix.html` |
| F2 | **No existe comando de inicialización.** El prefijo se crea automáticamente en la primera ejecución: `checkPrefixDir()` falla y se llama a `setupPrefix()`. | `darling-prefix.html`, `darling.c` |
| F3 | `darling shell [comando…]` arranca el contenedor o se conecta al que ya corre, inicializa el prefijo si hace falta y ejecuta el comando dentro. | `darling-shell.html`, `darling.c` |
| F4 | `darling shutdown` termina el contenedor **y todos los procesos que corren dentro**. | `darling-prefix.html` |
| F5 | No hay comando documentado para consultar si el contenedor está vivo. | ausencia documentada |
| F6 | El prefijo es overlayfs sobre el root de Darling dentro de un mount namespace de usuario; el filesystem superior debe soportar overlayfs. | `internals/basics/containerization.html`, `common-issues.html` |

## Decisión

La plataforma modela el ciclo de vida **solo con operaciones que Darling tiene**, y
nombra la diferencia en lugar de esconderla:

1. **No hay `init`.** `dg prefix start` ejecuta `darling shell /usr/bin/true` con
   `DPREFIX` exportado al proceso hijo. Ese único paso crea el prefijo si no existe
   (F2) y asegura el contenedor (F3).
2. **`start` no es idempotente por contrato de Darling**, es idempotente por
   consecuencia: si el contenedor ya corre, Darling se conecta y el comando termina
   igual.
3. **`stop` es `darling shutdown`** y es la operación más destructiva del ciclo de
   vida (F4): la plataforma la bloquea cuando hay procesos gestionados activos.
4. **La plataforma persiste su propia fase.** Como no se puede preguntar a Darling
   si el contenedor está vivo (F5), el estado conocido vive en
   `~/.local/share/darling-platform/state.json`, y una fase que quedó a mitad de
   transición (`Initializing`, `Starting`, `Stopping`) se reporta como `Error` con
   la operación interrumpida, en lugar de adivinar.
5. **La plataforma no estructifica el prefijo.** No crea directorios internos: los
   crea Darling (F2). La validación se limita a lo que la plataforma puede afirmar
   sin inventar: existencia, tipo, permisos, propiedad, longitud de ruta y que no
   sea el prefijo convencional del usuario.
6. **`DPREFIX` se exporta al proceso hijo**, nunca se pasa como asignación en línea
   de un comando (F1).
7. **El filesystem superior debe soportar overlayfs** (F6). La plataforma lo
   documenta y lo reporta como advertencia de diagnóstico; no intenta montar nada
   por su cuenta.

## Alternativas consideradas

| Alternativa | Por qué no |
| --- | --- |
| Simular `init` esperando un comando `darling prefix init` | No existe; el diseño se rompería en la primera ejecución real |
| Crear la estructura del prefijo a mano | Reemplaza una decisión de Darling y se rompe en cada actualización |
| Pasar `DPREFIX` como prefijo de un comando (`DPREFIX=x darling …`) | La documentación lo desaconseja explícitamente (F1) |
| Preguntar a Darling si el contenedor corre | No hay comando documentado (F5) |
| Usar `darling shutdown` sin guarda de procesos | Mata aplicaciones en ejecución sin aviso (F4) |

## Consecuencias

### Positivas

- Cada operación del gestor corresponde a una capacidad real de Darling.
- El comando de arranque también sirve como sonda de disponibilidad del runtime.
- El estado interrumpido es visible en vez de silencioso.

### Negativas y riesgos

- Sin comando de estado en Darling, la fase persistida puede quedar desfasada si el
  contenedor se detiene por fuera de la plataforma. Se mitiga con `validate` y con
  un mensaje explícito, no con optimismo.
- El `start` depende de que exista un binario `darling` funcional: en una máquina
  sin Darling, la operación falla con un error tipado `DarlingUnavailable`.
- Los comandos se verificaron contra la documentación y el código de la versión
  actual; **la prueba en una instalación real sigue pendiente** y es prerequisito de
  la Fase 0. Si upstream cambia `shell` o `shutdown`, el adaptador es el único lugar
  a corregir.

## Seguimiento

- Ejecutar `dg prefix start/stop` contra un Darling real y registrar versión o
  commit, según el criterio de salida de la Fase 0.
- Revisar el adaptador en cada actualización de la versión fijada de Darling.
- Si upstream agrega un comando de estado, reemplazar la fase persistida por la
  consulta directa y marcar este ADR como reemplazado.
