# Third-Party Licenses & Legal Disclaimers

Specter is open-source software licensed under the **MIT License**.

This document provides a comprehensive breakdown of third-party software, dependencies, and legal compliance considerations.

---

## 1. FFmpeg Compliance Notice

Specter interacts with [FFmpeg](https://ffmpeg.org/) as an external command-line tool.

* **Subprocess Isolation:** Specter does **not** link statically or dynamically against FFmpeg libraries (`libavcodec`, `libavformat`, `libavutil`, etc.). No C/C++ ABI linking is performed.
* **Inter-Process Communication:** Communication occurs exclusively through standard operating system pipes (`stdout` pipe:1) and process execution (`std::process::Command`).
* **License:** FFmpeg is licensed under the **GNU Lesser General Public License (LGPL) version 2.1 or later**, or the **GNU General Public License (GPL) version 2 or later** depending on compile-time configuration flags (e.g., when compiled with `--enable-gpl` for `libx264`).
* Under GNU licensing guidelines, executing an unmodified FFmpeg binary as an independent external child process over standard I/O does not create a combined or derivative work, preserving the permissive MIT licensing of Specter itself.
* If you distribute a pre-packaged build of FFmpeg alongside Specter, you must ensure that your distribution complies with FFmpeg's LGPL/GPL terms, including providing access to FFmpeg source code and applicable licenses upon request.
* *FFmpeg is a trademark of Fabrice Bellard, originator of the FFmpeg project.*

---

## 2. Hardware Video Codec Patents & Trademarks

Specter coordinates video encoding using hardware silicon acceleration blocks present on user GPUs:

* **NVIDIA NVENC:** Proprietary hardware video encoder silicon built into NVIDIA GeForce and Quadro GPUs. Invoked via FFmpeg's hardware wrappers.
* **AMD AMF (Advanced Media Framework):** Hardware encoder silicon on AMD Radeon graphics cards.
* **Intel Quick Sync Video (QSV):** Dedicated hardware encoding cores on Intel CPUs and Arc GPUs.
* **Patent Pools:** Video compression standards such as **H.264 (AVC)** and **H.265 (HEVC)** are subject to patent licensing pools managed by entities such as MPEG LA and Via Licensing. Hardware GPU encoders are pre-licensed by hardware vendors (NVIDIA, AMD, Intel) for personal consumer playback and recording. Users deploying Specter for commercial broadcast or paid streaming distribution are responsible for complying with any applicable MPEG LA or Via Licensing terms in their jurisdiction.

---

## 3. Rust Backend Dependencies

All Rust crates utilized in the Specter core are licensed under permissive open-source licenses (MIT or Apache-2.0):

| Crate | License | Purpose |
|---|---|---|
| `tauri` | MIT or Apache-2.0 | Native application runtime and window management |
| `tauri-plugin-fs` | MIT or Apache-2.0 | File system integration |
| `tauri-plugin-shell` | MIT or Apache-2.0 | Shell and child process execution |
| `tauri-plugin-dialog` | MIT or Apache-2.0 | Native OS folder picker dialogs |
| `tauri-plugin-global-shortcut` | MIT or Apache-2.0 | Windows global hotkey hooks |
| `serde` / `serde_json` | MIT or Apache-2.0 | Serialization for settings persistence |
| `tokio` | MIT | Async runtime |
| `anyhow` | MIT or Apache-2.0 | Structured error handling |
| `dirs` | MIT or Apache-2.0 | Standard Windows user path resolution |

---

## 4. Frontend Web Dependencies

All JavaScript / TypeScript libraries utilized in the frontend are licensed under permissive open-source licenses:

| Package | License | Purpose |
|---|---|---|
| `react` | MIT | User interface component architecture |
| `react-dom` | MIT | DOM rendering engine |
| `vite` | MIT | Frontend development and bundling toolchain |
| `typescript` | Apache-2.0 | Type checking and language services |
| `@tauri-apps/api` | MIT or Apache-2.0 | Frontend IPC bindings to Tauri core |

---

## 5. UI Assets, Icons, and Typography

* **Application Icon:** Custom pixel-art Specter ghost artwork created specifically for this project.
* **UI Vectors & Glyphs:** Built with custom SVG paths adhering to permissive Feather/Lucide design patterns (MIT License).
* **Typography:** Utilizes native operating system system fonts (`Segoe UI`, `-apple-system`, `Inter`, `system-ui`). No commercial webfonts are bundled.
