# Contribuir

Gracias por querer sumar. Este proyecto toca el sistema de archivos del usuario,
un entorno de compatibilidad compartido y archivos que no son confiables: por eso
el flujo es explícito y conservador.

Antes de escribir código, leé
[`PLAN_DARLING_PLATFORM.md`](PLAN_DARLING_PLATFORM.md) y los
[ADR](docs/architecture/decisions/). Las reglas de diseño del plan no son
sugerencias.

## Antes de abrir un cambio

- **Buscá o abrí un issue.** Describí el problema, el alcance y el criterio de
  salida. Sin criterio de salida no hay forma de saber si el cambio terminó.
- **Consultá primero** si el cambio toca alguno de estos puntos:
  - contratos de IPC o de datos ya consumidos;
  - el ciclo de vida del DPREFIX o sus locks;
  - el modelo de seguridad (validación de rutas, permisos, consentimiento);
  - una dependencia nueva;
  - una regla del plan o un ADR aceptado.
- **No prometas compatibilidad.** "Funciona con esta app" requiere evidencia
  reproducible, versión del entorno y hardware documentados.

## Flujo

1. Fork y rama desde `main`: `feat/…`, `fix/…`, `docs/…` o `ci/…`.
2. Commits en [Conventional Commits](https://www.conventionalcommits.org/es/v1.0.0/),
   en inglés, pequeños y revisables. Un cambio de comportamiento por commit, con
   sus pruebas y su documentación.
3. Verificá localmente lo mismo que corre CI:

   ```bash
   scripts/dev/env-check.sh
   scripts/ci/check.sh
   ```

4. Abrí el PR completando la checklist de abajo.
5. Atendé la revisión. Los cambios de alcance se discuten en el issue, no en el
   diff.

Si tu cambio supera las ~400 líneas modificadas, considerá partirlo en PRs
encadenados. Un PR que no se puede revisar con atención no se revisa.

## Checklist de PR

Copiá esto en la descripción y completalo:

```markdown
### Problema
Qué falla o qué falta, con evidencia.

### Alcance
Componentes y archivos afectados. Qué queda explícitamente afuera.

### Decisiones
Decisiones técnicas relevantes y alternativas descartadas. ADR si cambia una
regla.

### Pruebas
Comandos ejecutados y resultado. Si algo no se pudo probar, decilo.

### Cambios de contrato
Esquema, API, IPC, formato de datos. "Ninguno" si no aplica.

### Riesgos y límites
Qué puede fallar y qué no se verificó todavía.
```

## Reglas de diseño que aplican a todo cambio

- Reutilizá Darling; no reimplementes sus subsistemas.
- Un único DPREFIX compartido y dedicado. Nunca un prefijo por aplicación.
- Sin root para operaciones ordinarias. Sin modificar drivers ni la configuración
  del host automáticamente.
- Bundles, ZIP, DMG, PKG, `Info.plist` y cabeceras Mach-O son **entrada no
  confiable**: validá rutas, rechazá path traversal y symlinks maliciosos, y no
  ejecutes instaladores sin consentimiento explícito.
- El daemon acepta operaciones tipadas y validadas; nunca comandos de shell
  arbitrarios.
- Detectá con evidencia parcial y tolerante a fallos; no inventes capacidades.
- No agregues abstracciones sin consumidor real ni dependencias sin justificar.

## Seguridad

No abras un issue público para reportar una vulnerabilidad. Seguí
[`SECURITY.md`](SECURITY.md).

## Conducta

Este proyecto adopta el [Código de Conducta](CODE_OF_CONDUCT.md). Al participar,
lo aceptás.

## Licencia de las contribuciones

Al enviar un cambio aceptás publicarlo bajo **GPL-3.0-or-later**, la licencia del
repositorio. No incluyas código con licencia incompatible; si necesitás integrar
algo de Darling, revisá primero su licencia y proponelo en el issue.
