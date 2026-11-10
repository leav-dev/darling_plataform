# Reporte de hardware

`dp hardware scan` describe el host con un reporte versionado. Este documento fija
qué significa cada campo, cuándo falta, qué códigos de diagnóstico existen y qué
no puede afirmar el escaneo.

- **Crate:** `dp-hardware-intelligence` (`modules/hardware-intelligence/src/model.rs`).
- **Comando:** `dp hardware scan` (texto para humanos) y `dp hardware scan --json`.
- **Versionado:** `schema_version = "1.0"`.
- **Códigos de salida:** `0` cuando el host quedó descrito (`complete` o `partial`);
  `1` cuando `scan_status` es `failed`; `2` para errores de uso.

El escaneo es de solo lectura: lee `/etc/os-release`, `/proc` y `/sys` a través del
trait `HostSource` (`read`, `read_dir`, `exists`, `read_link`), no necesita root y
no escribe nada en el host.

## Forma general

```json
{
  "schema_version": "1.0",
  "scan_status": "complete",
  "system": {
    "architecture": "x86_64",
    "kernel": "6.8.0-45-generic",
    "distribution": {
      "id": "ubuntu",
      "name": "Ubuntu",
      "version_id": "24.04",
      "pretty_name": "Ubuntu 24.04.1 LTS",
      "id_like": ["debian"],
      "support": "unknown"
    },
    "libc": { "family": "glibc", "evidence": "/lib/x86_64-linux-gnu/libc.so.6" }
  },
  "cpu": {
    "architecture": "x86_64",
    "vendor": "AuthenticAMD",
    "model": "AMD Ryzen 7 7840U w/ Radeon 780M Graphics",
    "logical_cores": 16,
    "physical_cores": 8,
    "sockets": 1,
    "features": ["avx2", "sse4_2"]
  },
  "gpus": [
    {
      "pci_address": "0000:03:00.0",
      "vendor_id": "0x1002",
      "device_id": "0x15bf",
      "class": "0x030000",
      "driver": "amdgpu"
    }
  ],
  "memory": { "total_bytes": 33403977728, "available_bytes": 20000000000 },
  "displays": [
    {
      "connector": "card0-eDP-1",
      "status": "connected",
      "enabled": true,
      "resolution": "2880x1800"
    }
  ],
  "diagnostics": [
    {
      "provider": "vulkan",
      "level": "info",
      "code": "not_consulted",
      "message": "Vulkan enumeration is not part of this scan; ..."
    }
  ]
}
```

## Campos

### Raíz

| Campo | Tipo | Significado | Cuándo falta |
| --- | --- | --- | --- |
| `schema_version` | string | Identifica la forma del JSON | Nunca |
| `scan_status` | `complete` \| `partial` \| `failed` | Cuánto del host se pudo describir | Nunca |
| `system` | objeto | Identidad del sistema operativo | Nunca (ver más abajo) |
| `cpu` | objeto | Descripción de la CPU | `/proc/cpuinfo` ilegible o sin procesadores |
| `gpus` | array | GPUs PCI descubiertas | Nunca; puede venir vacío |
| `memory` | objeto | Memoria física | `/proc/meminfo` ilegible o sin `MemTotal` |
| `displays` | array | Conectores DRM que publica el kernel | Nunca; puede venir vacío |
| `diagnostics` | array | Hallazgos por subsistema | Nunca; puede venir vacío |

`system` nunca es `null`: si ningún dato de identidad se pudo leer, el reporte
llega igual con `scan_status: "failed"` y los motivos en `diagnostics`.

`scan_status` se deriva del contenido, no de una intención:

- `failed`: `system.distribution`, `system.kernel` y `system.libc` están los tres
  ausentes; el host no se pudo describir.
- `partial`: el host se describió, pero algún proveedor falló o algún diagnóstico
  tiene nivel `warning` o `error`.
- `complete`: todo proveedor respondió y los diagnósticos son solo informativos
  (el aviso de Vulkan no degrada nada).

### `system`

| Campo | Tipo | Significado | Cuándo falta |
| --- | --- | --- | --- |
| `architecture` | string | Arquitectura del binario (`std::env::consts::ARCH`), no la del host | Nunca |
| `kernel` | string | Release de `/proc/sys/kernel/osrelease`, sin espacios finales | Archivo ilegible o vacío |
| `distribution` | objeto | Identidad declarada en `/etc/os-release` | Archivo ilegible o sin `ID` |
| `libc` | objeto | Familia de libc con la evidencia que la sostiene | No se encontró ningún loader ni `libc.so.6` |

`architecture` es una propiedad del binario que ejecuta el escaneo. Un binario
cruzado describe el host que lo ejecuta, no el host para el que se compiló.

### `system.distribution`

| Campo | Tipo | Significado | Cuándo falta |
| --- | --- | --- | --- |
| `id` | string | Identificador máquina (`ubuntu`, `arch`, `fedora`) | Nunca: sin `ID` no hay distribución |
| `name` | string | Nombre corto publicado por la distro | `NAME` ausente o vacío |
| `version_id` | string | Versión (`24.04`, `rolling`) | `VERSION_ID` ausente o vacío |
| `pretty_name` | string | Nombre completo publicado por la distro | `PRETTY_NAME` ausente o vacío |
| `id_like` | array | Distros compatibles declaradas, en el orden publicado | Nunca; puede venir vacío |
| `support` | `supported` \| `partial` \| `unsupported` \| `unknown` | Política de soporte de la plataforma | Nunca |

`support` es siempre `unknown` en este corte: describir hardware no es prometer
soporte, y la política de compatibilidad llega con una fase posterior.

### `system.libc`

| Campo | Tipo | Significado | Cuándo falta |
| --- | --- | --- | --- |
| `family` | `glibc` \| `musl` \| `unknown` | Familia de la C library | Nunca, dentro de `libc` |
| `evidence` | string | Ruta que probó la identificación | Nunca, dentro de `libc` |

`unknown` está declarado en el vocabulario pero hoy no se emite: cuando no se
identifica la familia, el objeto `libc` completo se omite y queda el diagnóstico
`libc_unidentified` en su lugar.

### `cpu`

| Campo | Tipo | Significado | Cuándo falta |
| --- | --- | --- | --- |
| `architecture` | string | Arquitectura del binario | Nunca |
| `vendor` | string | `vendor_id` de `/proc/cpuinfo` | La arquitectura no lo publica (por ejemplo AArch64) |
| `model` | string | `model name` de `/proc/cpuinfo` | El kernel no lo publica |
| `logical_cores` | número | Cantidad de procesadores lógicos (una estrofa `processor` cada uno) | Nunca; sin procesadores la CPU no se describe |
| `physical_cores` | número | Sockets x núcleos por socket | No hay `physical id` o `cpu cores` |
| `sockets` | número | Cantidad de `physical id` distintos | El kernel no publica topología |
| `features` | array | Unión ordenada de los flags de todas las estrofas | Nunca; puede venir vacío |

Un `model` ausente no invalida la CPU: se emite un diagnóstico informativo y el
reporte identifica el procesador por su arquitectura.

### `memory`

| Campo | Tipo | Significado | Cuándo falta |
| --- | --- | --- | --- |
| `total_bytes` | número | `MemTotal` convertido de KiB a bytes | Nunca; sin `MemTotal` no hay objeto `memory` |
| `available_bytes` | número | `MemAvailable` en bytes | El kernel no publica `MemAvailable` |
| `swap_total_bytes` | número | `SwapTotal` en bytes | El kernel no publica `SwapTotal` |

Los campos se buscan por nombre, nunca por posición.

### `gpus[]`

| Campo | Tipo | Significado | Cuándo falta |
| --- | --- | --- | --- |
| `pci_address` | string | BDF (`0000:03:00.0`); identificador estable entre escaneos | Nunca |
| `vendor_id` | string | Identificador PCI de vendor (`0x1002`) | Nunca en una GPU listada |
| `device_id` | string | Identificador PCI de dispositivo (`0x15bf`) | Nunca en una GPU listada |
| `class` | string | Clase PCI (`0x030000` es controlador de video) | Nunca en una GPU listada |
| `driver` | string | Nombre del driver de kernel enlazado | El dispositivo no tiene driver enlazado |

Solo se listan dispositivos cuya clase empieza con `0x03`. Una GPU sin driver
enlazado es normal y no genera diagnóstico: el módulo puede no estar cargado. Las
GPUs se ordenan por `pci_address`.

### `displays[]`

| Campo | Tipo | Significado | Cuándo falta |
| --- | --- | --- | --- |
| `connector` | string | Nombre del conector (`card0-DP-1`) | Nunca |
| `status` | `connected` \| `disconnected` \| `unknown` | Estado medido por el kernel | Nunca; un `status` ilegible queda `unknown` |
| `enabled` | booleano | Si el kernel habilitó el conector | No se publica o el valor no es `enabled`/`disabled` |
| `resolution` | string | Primer modo que anuncia el conector (`1920x1080`) | El conector no anuncia modos (típico en desconectados) |

### `diagnostics[]`

| Campo | Tipo | Significado |
| --- | --- | --- |
| `provider` | `system` \| `cpu` \| `memory` \| `gpu` \| `display` \| `vulkan` | Subsistema al que pertenece el hallazgo |
| `level` | `info` \| `warning` \| `error` | Cuánto afecta al reporte |
| `code` | string | Código máquina estable, apto para scripts |
| `message` | string | Explicación humana que nombra la ruta o el paso siguiente |

## Reglas de serialización

Los consumidores pueden apoyarse en estas cuatro reglas; están fijadas por pruebas
en `model.rs`:

1. **Enums en `snake_case`.** Se serializa el nombre máquina, nunca el texto de
   presentación: `"complete"`, `"connected"`, `"glibc"`, `"unknown"`.
2. **`None` se omite, no se serializa como `null`.** Un subsistema ausente no
   aparece como clave. Un reporte parcial queda más chico y sin ambigüedad: la
   ausencia de `cpu` significa exactamente "no se pudo describir la CPU", y el
   diagnóstico dice por qué. Ningún objeto del reporte contiene `null`.
3. **Las colecciones siempre están presentes.** `gpus`, `displays`, `diagnostics`
   y `id_like` son arrays, posiblemente vacíos. Un consumidor itera sin comprobar
   `null`.
4. **`schema_version` identifica la forma.** Es siempre el primer dato que un
   consumidor debe leer; el valor actual es `"1.0"`.

Serializar el reporte como JSON termina con un salto de línea, igual que el resto
de las salidas de `dp`.

## Versionado

- Agregar campos opcionales o valores nuevos en enums mantiene `"1.0"`.
- Quitar un campo, renombrarlo o cambiarle el significado sube el número mayor
  (`"2.0"`), para que un consumidor pueda rechazar un reporte que no entiende en
  lugar de interpretarlo mal.
- Un consumidor debe comparar el prefijo mayor y rechazar lo desconocido; nunca
  asumir que un reporte futuro se parece al actual.

## Diagnósticos

Todos los códigos que el escaneo puede emitir hoy:

| Código | Proveedor | Nivel | Significado |
| --- | --- | --- | --- |
| `source_unreadable` | cualquiera | `error` | La ruta requerida por el proveedor no se pudo leer; el mensaje nombra la ruta y el error |
| `kernel_unavailable` | `system` | `warning` | `/proc/sys/kernel/osrelease` no se pudo leer o está vacío; el kernel queda ausente |
| `distribution_unidentified` | `system` | `warning` | `/etc/os-release` se leyó pero no declara `ID`; no hay distribución que reportar |
| `libc_unidentified` | `system` | `info` | No se encontró `ld-musl-<arch>.so.1` ni `libc.so.6`; la familia de libc queda ausente |
| `cpu_model_unavailable` | `cpu` | `info` | El kernel no publica `model name`; la CPU se identifica por arquitectura |
| `cpuinfo_empty` | `cpu` | `error` | `/proc/cpuinfo` no lista ningún `processor`; no hay objeto `cpu` |
| `mem_total_missing` | `memory` | `error` | `/proc/meminfo` no tiene `MemTotal`; no hay objeto `memory` |
| `device_class_unreadable` | `gpu` | `warning` | Un dispositivo PCI no publica `class`; se omite del listado de GPUs |
| `device_identity_incomplete` | `gpu` | `warning` | Un controlador de video no publica `vendor` y `device`; se omite |
| `status_unreadable` | `display` | `warning` | Un conector no publica `status`; queda con estado `unknown` |
| `no_drm_connectors` | `display` | `info` | `/sys/class/drm` no publica conectores; el host puede ser headless |
| `not_consulted` | `vulkan` | `info` | Vulkan no se consultó en este corte; la ausencia de sección Vulkan no invalida el reporte |

Reservado: `unexpected_format` (`warning`) está declarado en `source.rs` como
ayuda para "la ruta existe pero no tiene los campos esperados". Ningún proveedor
lo emite todavía; no es un código que un consumidor vaya a ver hoy.

Todos los mensajes están en inglés porque forman parte del contrato de salida del
CLI, que es inglés; la documentación de arquitectura es española.

## Limitaciones

- **La familia de libc es una heurística sobre rutas de loader y objetos
  compartidos.** Se busca `ld-musl-<arch>.so.1` y después `libc.so.6` en las rutas
  habituales. Si el sistema usa una ruta no contemplada, el resultado es ausente,
  no incorrecto. El campo `evidence` publica la ruta que probó la identificación
  para que la afirmación sea verificable.
- **`resolution` es el primer modo que anuncia el conector**, es decir el modo
  preferido, no el que eligió el compositor. Un servidor gráfico puede estar
  mostrando otra cosa.
- **Las features de CPU son la unión ordenada de los flags de `/proc/cpuinfo`.**
  Describen lo que el kernel publica para esos procesadores, no lo que cada
  aplicación puede usar en su contexto.
- **La topología puede faltar.** En hosts que no publican `physical id` (por
  ejemplo AArch64) `physical_cores` y `sockets` quedan ausentes; el resto de la
  CPU se describe igual.
- **Las GPUs se enumeran solo desde sysfs.** Si `/sys/bus/pci/devices` no está
  disponible (contenedor sin sysfs montado, por ejemplo) el reporte no lista GPUs
  y agrega `source_unreadable`. Una GPU ausente del reporte no prueba que no
  exista.
- **Vulkan no se consulta en este corte.** El reporte no trae dispositivos,
  extensiones, propiedades ni features Vulkan, y lo dice explícitamente con el
  diagnóstico informativo `not_consulted`. Enumerar Vulkan requiere cargar el
  loader dinámicamente y probar sobre hardware real, así que entra como corte
  propio, con su ADR.
- **Los campos `enabled` y `resolution` de un conector pueden desaparecer sin
  diagnóstico.** Son datos accesorios: si el kernel no los publica, el conector se
  describe con lo que sí hay. Los fallos que sí importan (`status`) sí se
  diagnostican.
- **El escaneo nunca necesita root y nunca escribe en el host.** Solo lee rutas
  absolutas del sistema de archivos y resuelve enlaces simbólicos de sysfs.
- **`dp hardware scan` describe; no decide.** No elige GPU ni backend, no instala
  drivers y no modifica configuración: eso es política del runtime (ADR-0004,
  ADR-0005).

## Reproducibilidad

Los proveedores no leen el host directamente: reciben un `HostSource`. La
implementación real es `LinuxHost`; `MappingSource` reproduce un host capturado
con archivos, directorios y enlaces en memoria, y es lo que usan las pruebas. Por
eso un cambio en los parsers se puede probar con texto de fixtures y sin tocar la
máquina que corre los tests.
