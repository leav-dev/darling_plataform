# ADR-0010: Instalador de un comando entregado como paquete `.deb`

- **Estado:** aceptada
- **Fecha:** 2026-11-04
- **Responsables:** mantenedores de Darling Platform
- **Reemplaza a:** —
- **Reemplazada por:** —

## Contexto

El pedido es instalar todo con un solo comando. La plataforma convive con dos
artefactos de naturaleza distinta: Darling, que es de upstream y se instala como
binario **setuid root** desde un release pineado (ADR-0009), y `dp`, que es código
propio y debe poder actualizarse y desinstalarse como cualquier programa del
sistema.

El plan pide que los paquetes no dependan de rutas de desarrollo y que la
instalación se pueda reproducir en entornos limpios (Fase 8), y exige
consentimiento antes de ejecutar instaladores que cambian el sistema.

## Decisión

1. **El proyecto se empaqueta como `.deb`** (`darling-platform`), construido de
   forma reproducible con `dpkg-deb --root-owner-group`: payload mínimo
   (`usr/bin/dp`, copyright y changelog), dueño `root:root`, versión tomada del
   workspace (`0.1.0`) con revisión de empaquetado (`0.1.0-1`).
2. **La instalación pasa por `apt`**, no por copiar archivos: resuelve
   dependencias, permite `apt upgrade` y `apt remove` reales.
3. **Se entrega un repositorio apt local reproducible** (`dpkg-scanpackages` +
   `Release` con hashes, firma GPG opcional). Publicar un repositorio remoto
   firmado requiere hosting y una clave de larga vida: queda para la Fase 8.
4. **El instalador corre sin privilegios y eleva por paso.** Avisa qué necesita
   root y por qué, pide la contraseña una vez con `sudo -v`, ejecuta solo los dos
   pasos elevados (`dpkg` de Darling y `apt-get install` del paquete propio) y no
   deja ninguna shell ni proceso con privilegios. Si se lo invoca como root, se
   niega.
5. **`--dry-run` imprime cada comando** —incluidos los `sudo`— y no ejecuta nada.
   `--yes` existe para instalaciones desatendidas; sin él, cada paso elevado se
   confirma.
6. **No se toca nada fuera de los dos paquetes**: no se agregan repositorios de
   terceros, no se ejecuta `apt upgrade`, no se modifican drivers ni configuración
   del host.

## Alternativas consideradas

| Alternativa | Por qué no |
| --- | --- |
| `curl … \| bash` con copia de archivos | Sin actualización ni desinstalación reales; y ejecutar un script remoto sin revisarlo es exactamente lo que el plan prohíbe |
| `cargo install --path apps/cli` | Esconde el binario en `~/.cargo/bin`, sin integración de sistema ni versión de paquete |
| Repositorio remoto firmado desde el primer día | Requiere hosting, clave GPG de larga vida y CI de release: Fase 8 |
| Empaquetar también Darling | Duplica trabajo de upstream, hereda su licencia y su cadencia de releases |
| Correr todo el instalador con `sudo` | Escala privilegios para tareas que no los necesitan y deja una shell root |

## Consecuencias

### Positivas

- Un solo comando, con `apt` haciendo lo que sabe hacer.
- `apt upgrade`, `apt remove` y auditoría del paquete son estándar.
- El paquete es verificable sin root: se construye, se inspecciona con
  `dpkg-deb` y sus pruebas corren en CI.

### Negativas y riesgos

- `Depends: libc6 (>= 2.39)` ata el paquete a glibc de Ubuntu 24.04; ampliar la
  matriz de distribuciones obliga a revisar ese mínimo o a empaquetar por
  distro.
- Un repositorio local sin firma **no debe publicarse**; el script lo advierte y
  la firma es opcional, no obligatoria.
- Instalar Darling sigue requiriendo root y su binario es setuid: el riesgo es de
  upstream (ADR-0009), no se mitiga acá.
- El desinstalador no borra el DPREFIX ni las aplicaciones: es deliberado, y hay
  que documentarlo para que nadie lo interprete como un fallo.

## Seguimiento

- Firmar y publicar el repositorio cuando exista infraestructura de release.
- Revisar el mínimo de `libc6` al agregar la primera distribución fuera de Ubuntu
  24.04.
- Agregar `apt-get purge` y limpieza de DPREFIX como decisión explícita, no por
  defecto.
