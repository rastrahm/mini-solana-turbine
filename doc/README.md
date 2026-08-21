# Documentación de diagramas

Diagramas del crate `mini-solana-turbine`. Formato [Mermaid](https://mermaid.js.org/): se ven en GitLab, GitHub y en la vista previa de Markdown de Cursor.

| Archivo | Contenido |
| --- | --- |
| [flujograma.md](flujograma.md) | Arquitectura: de UDP al fanout Turbine |
| [diagrama-clases.md](diagrama-clases.md) | Tipos y relaciones entre módulos |
| [diagrama-flujo.md](diagrama-flujo.md) | Flujo detallado de ingestión y de `--debug-udp` |

Features: `uring` (UDP `io_uring`), `simd` (FEC + `Pipeline`). Default = ambas.
