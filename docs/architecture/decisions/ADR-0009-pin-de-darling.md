# ADR-0009: Pin de Darling y verificación de instalación

- **Estado:** aceptada
- **Fecha:** 2026-11-04
- **Responsables:** mantenedores de Darling Platform
- **Reemplaza a:** —
- **Reemplazada por:** —

## Contexto

La plataforma administra un entorno Darling cuyo comportamiento cambia entre
versiones: interfaces internas, comandos, disposición del prefijo y el propio
contenedor. Diagnosticar un fallo sin saber contra qué Darling se ejecutó es
imposible, y el plan exige "fijar y documentar la versión o commit de Darling
utilizado para cada release" (ADR-0006).

Tres hechos verificados acotan las opciones:

| Hecho | Fuente |
| --- | --- |
| Upstream publica **snapshots fechados de `master`** (`v0.1.AAAAMMDD`), no una línea semver. La última release estable es `v0.1.20260608`, commit `e947f0d5a3c6bba27e3631d9bf0da2a9840e6894`, con paquetes Debian y archivo de fuentes publicados junto a hashes. | API de GitHub, `darlinghq/darling` releases/tags |
| El binario `darling` **exige setuid root**: `darling.c` aborta con `missingSetuidRoot()` si `geteuid() != 0`. | `src/startup/darling.c` |
| No hay comando documentado que reporte la versión instalada (`darling` acepta `shell` y `shutdown`; sin argumentos muestra ayuda). | `darling.c`, `docs.darlinghq.org` |

## Decisión

1. **Se pinea una release estable, nunca una rama.** Versión fijada:
   `v0.1.20260608`, commit `e947f0d5a3c6bba27e3631d9bf0da2a9840e6894`.
2. **Un solo archivo de verdad:** `deps/darling.toml` con versión, commit, fecha,
   URL del tag, assets con bytes y sha256, y el registro de parches propios.
3. **El pin se embebe en el binario** (`include_str!` en
   `modules/shared-prefix/src/pin.rs`) y se reporta en `dg prefix status` y en su
   salida `--json`. Una prueba fija los valores: subir la versión es un cambio de
   código revisable, no una deriva silenciosa.
4. **La instalación se verifica antes, no se adivina después.** El script
   `scripts/dev/install-darling-pin.sh` descarga el asset del release, comprueba el
   sha256 registrado y solo instala con `--yes`. No se inventa una detección de
   versión instalada, porque no existe un mecanismo documentado que la exponga.
5. **Los parches propios se registran** en la sección `[patches]` del pin con su
   motivo y su estado upstream (ADR-0006).

## Alternativas consideradas

| Alternativa | Por qué no |
| --- | --- |
| Seguir `master` | Un objetivo móvil no se puede diagnosticar ni reproducir |
| Detectar la versión instalada en tiempo de ejecución | No hay comando que la reporte; se implementaría una adivinanza |
| Usar el paquete de la distribución | No hay garantía de versión y rompe la reproducibilidad del diagnóstico |
| Duplicar la versión en código y en documentación | Dos fuentes que se desincronizan; por eso el pin es un archivo embebido |

## Consecuencias

### Positivas

- Todo reporte de compatibilidad queda atado a una versión concreta y verificable.
- El hash del asset detecta una descarga corrupta o manipulada antes de instalar.
- Subir de versión obliga a actualizar una prueba: el cambio se revisa.

### Negativas y riesgos

- **`darling` es setuid root.** La plataforma no necesita privilegios, pero invoca
  un binario que sí los tiene: el riesgo queda del lado de Darling y debe
  documentarse en `SECURITY.md`, no esconderse.
- Nada impide que un usuario instale otro Darling manualmente; el pin describe lo
  esperado, no lo impone en tiempo de ejecución.
- Las releases son snapshots: "estable" significa probada por upstream, no una
  garantía de compatibilidad con nuestras aplicaciones.

## Seguimiento

- Probar `start`/`stop` contra esta versión (Fase 0) y registrar el resultado.
- Revisar el pin en cada release de Darling; documentar breaking changes antes de
  subir.
- Si upstream agrega un comando de versión, sumar detección y comparar con el pin
  en `dg prefix status`.
