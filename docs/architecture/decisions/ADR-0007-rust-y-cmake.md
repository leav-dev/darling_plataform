# ADR-0007: Rust para los módulos y CMake para el build

- **Estado:** aceptada
- **Fecha:** 2026-11-04
- **Responsables:** mantenedores de Darling Platform
- **Reemplaza a:** —
- **Reemplazada por:** —

## Contexto

El proyecto convive con Darling, que es C++ y se construye con CMake, y con una
capa propia de administración, análisis y descubrimiento de hardware.

Esa capa necesita: parseo de binarios y archivos no confiables (Mach-O,
`Info.plist`, PLIST binario), tipos de dominio estables, concurrencia segura y
detección de hardware multiplataforma. También necesita integrarse con el build
nativo existente y con el empaquetado de distribuciones.

## Decisión

- **Rust** para los módulos de plataforma: modelos de dominio, DHI, prefijo,
  compatibilidad, catálogo, inspector e instalador, daemon y CLI.
- **CMake** coordina el proyecto completo: configuración, build, pruebas e
  instalación.
- **Cargo** compila y prueba los crates; CMake los invoca y consume sus artefactos.
- El build de Darling sigue siendo CMake y se integra como dependencia externa
  (ADR-0006).
- La versión mínima de Rust y el conjunto de compuertas (`fmt`, `clippy`, `test`)
  están fijados en los documentos del repositorio y en CI.

## Alternativas consideradas

| Alternativa | Por qué no |
| --- | --- |
| Todo en C++ con CMake | Consistente con Darling, pero mucho más costoso para parseo de archivos no confiables y para la concurrencia |
| Build puramente Cargo, sin CMake | Aísla el proyecto del ecosistema de paquetes de las distribuciones y de la integración con Darling |
| Un `CMakeLists.txt` por crate, sin Cargo | Pierde el ecosistema de crates y el modelo de dependencias |
| Otro lenguaje (Go, Python) | Empaquetado, integración nativa y control de memoria menos adecuados para el trabajo de bajo nivel |

## Consecuencias

### Positivas

- El compilador y Clippy hacen verificables las fronteras entre módulos.
- Parseo de entradas no confiables sin riesgo de desbordamientos por descuido.
- Un solo árbol de build produce binarios instalables y reproducibles.
- La integración con el empaquetado de distribuciones usa las herramientas
  estándar.

### Negativas y riesgos

- Dos toolchains (Rust y C++) en un mismo build: hay que documentar requisitos y
  versiones mínimas.
- El build de CMake tiene que delegar bien en Cargo: rutas de salida, perfiles y
  dependencias explícitas; nada de rutas hardcodeadas que fallen entre generadores.
- La vida útil de una versión antigua de Rust condiciona el empaquetado en
  distribuciones conservadoras (NixOS, Debian estable).

## Seguimiento

- Mantener actualizados los requisitos de toolchain en `DEVELOPMENT.md` y en CI.
- Verificar el build en generadores distintos (Ninja y Makefiles) antes de ampliar
  la matriz de plataformas.
- Revisar las versiones mínimas soportadas cuando las distribuciones objetivo lo
  exijan.
