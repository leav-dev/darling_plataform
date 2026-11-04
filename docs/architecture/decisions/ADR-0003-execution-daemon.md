# ADR-0003: Daemon de ejecución por sesión de usuario

- **Estado:** aceptada
- **Fecha:** 2026-11-04
- **Responsables:** mantenedores de Darling Platform
- **Reemplaza a:** —
- **Reemplazada por:** —

## Contexto

Lanzar y supervisar aplicaciones requiere estado que sobrevive al frontend:
procesos activos, estado del prefijo, locks y eventos. Si la interfaz lanza
procesos directamente, ese estado se pierde al cerrarla y no hay un lugar único que
decida si el entorno puede apagarse.

La alternativa de un daemon por aplicación multiplicaría procesos residentes y
volvería imposible coordinar el prefijo compartido.

## Decisión

Un **único daemon por sesión de usuario** —el Darling Execution Manager—,
activado bajo demanda:

- preferentemente sobre D-Bus de sesión, con unidad `systemd --user` cuando esté
  disponible;
- métodos tipados: `LaunchApp`, `StopApp`, `GetAppStatus`, `ListRunningApps`,
  `GetPrefixStatus`;
- eventos: `AppStarted`, `AppStopped`, `AppCrashed`, `PrefixStateChanged`;
- el daemon supervisa procesos, mantiene el prefijo activo mientras haya
  aplicaciones gestionadas y coordina el apagado;
- **el daemon no acepta comandos de shell**: recibe operaciones validadas, nunca
  rutas y argumentos arbitrarios que lo conviertan en un ejecutor genérico;
- no reimplementa `darlingserver`, `launchd` ni otros componentes internos de
  Darling.

Los nombres de servicio, objeto y métodos son propuesta hasta que exista una
interfaz estable y probada.

## Alternativas consideradas

| Alternativa | Por qué no |
| --- | --- |
| La interfaz lanza los procesos | Estado perdido al cerrar, sin coordinación del prefijo, sin supervisión |
| Un daemon por aplicación | Multiplica procesos residentes y no puede coordinar un prefijo compartido |
| Daemon de sistema (root) | La plataforma no necesita privilegios elevados para operaciones ordinarias |
| Solo `systemd --user` sin D-Bus | Asume systemd; el plan exige no asumir esa dependencia |

## Consecuencias

### Positivas

- Un solo dueño del estado de ejecución y del ciclo de vida del prefijo.
- El frontend puede reiniciarse sin perder procesos ni estado.
- La API tipada acota la superficie de ataque frente a un ejecutor de comandos.

### Negativas y riesgos

- Hay que versionar el protocolo IPC y sostener compatibilidad mientras evoluciona.
- Un crash del daemon deja procesos huérfanos: requiere recuperación de estado y
  logs.
- Depende de D-Bus de sesión o de una activación equivalente; en entornos sin
  systemd hay que prever el arranque manual.

## Seguimiento

- Definir el contrato IPC mínimo y su versionado antes de implementar el daemon.
- Probar el ciclo de vida completo: lanzar, consultar, cerrar y recuperar tras un
  crash.
- Documentar el arranque en entornos sin `systemd --user`.
