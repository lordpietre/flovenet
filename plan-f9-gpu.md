# Plan de Acción: Flovenet

## PRIORIDAD 0: Documentación de uso (completado ✅)

**Problema**: El usuario instaló Flovenet en Ubuntu pero no sabe cómo usarlo ni acceder al panel web.

**Solución implementada**: README reescrito con instrucciones paso a paso completas:
- Instalación desde código fuente
- Cómo iniciar los 3 servicios necesarios (daemon, gateway, dashboard)
- Cómo acceder al panel web (http://localhost:3000)
- Cómo registrarse y hacer login
- Ejemplos de uso de cada página (Dashboard, Feed, Profile, Network)
- Comandos CLI con explicaciones

**Archivos modificados**: `README.md`

---

## Plan F9: GPU Distribuida

### Estado actual (resumen)

| Capa | Estado | Detalle |
|------|--------|---------|
| Detección GPU (env vars) | ✅ | `FLOVENET_GPU_VRAM_GB` / `FLOVENET_GPU_MODEL` |
| Detección GPU (Linux NVIDIA) | ✅ | `/proc/driver/nvidia/gpus/*/information` |
| Detección GPU (Linux AMD) | ✅ | `/sys/class/drm/card*/device/mem_info_vram_total` |
| Detección GPU (Windows) | ✅ | `nvidia-smi` → `wmic` fallback |
| GpuSlot bin-packing (8/4/2 GiB) | ✅ | `gpu.rs` |
| Scheduler VRAM matching | ✅ | `can_accept()` valida `gpu_vram_gb` |
| AI role slot calculation | ✅ | `slots_for_role(Ai)` usa `vram_slots` |
| **JobOffer GPU field** | ✅ | `gpu_vram_gb` + `gpu_required` añadidos |
| **Daemon job handler GPU wiring** | ✅ | Usa `offer.gpu_vram_gb` y `offer.gpu_required` |
| **GPU slot lifecycle** | ✅ | `GpuSlotManager` con allocate/release |
| **GraphQL GPU state** | ✅ | Query `nodeResources` expone GPU info |
| **Tests de integración** | ✅ | Tests end-to-end en scheduler |
| Detección Intel | ❌ | Baja prioridad |
| Ejecución GPU real (CUDA/OpenCL) | ❌ | Fuera de scope para F9 |

---

## Tareas completadas ✅

### T1. Añadir campo GPU a `JobOffer` (market_protocol) ✅

**Archivo**: `market_protocol/src/lib.rs`

- Añadido `gpu_vram_gb: Option<f64>` a `JobOffer`
- Añadido `gpu_required: bool` a `JobOffer`
- Actualizado test `test_job_offer_serde` y añadido `test_job_offer_with_gpu`
- Protocol version bumped: `JOB_PROTOCOL` → `"/flovenet/job/1.1.0"`

### T2. Cablear GPU en el job handler del daemon ✅

**Archivo**: `daemon/src/main.rs`

- Cambiado `gpu_vram_gb: None` → `gpu_vram_gb: offer.gpu_vram_gb`
- Cuando `offer.gpu_required == true`, usa `NodeRole::Ai` en vez de `NodeRole::Compute`
- Pasado `gpu_vram_gb` al `Manifest`

### T3. Añadir `gpu_vram_gb` al `Manifest` (vm_runtime) ✅

**Archivo**: `vm_runtime/src/lib.rs`

- Añadido `gpu_vram_gb: Option<f64>` a `Manifest`
- Actualizados todos los tests

### T4. GPU slot lifecycle manager (resource_manager) ✅

**Archivo**: `resource_manager/src/gpu.rs`

- Creado `GpuSlotManager` struct con `Vec<GpuSlot>`
- Métodos: `allocate(required_gb) -> Result<Vec<u32>>`, `release(slot_ids: &[u32])`
- Tests: allocate, release, allocate-fails-when-full, release-then-allocate-again

### T5. Integrar GpuSlotManager en el daemon ✅

**Archivo**: `daemon/src/main.rs`

- Creado `GpuSlotManager` al inicio si hay GPU detectada (envuelto en `Arc<Mutex<>>`)
- En el job handler: allocate antes de aceptar, release al terminar (éxito o fracaso)
- Si allocate falla, rechaza con "insufficient GPU slots"

### T6. GraphQL: exponer estado GPU ✅

**Archivo**: `graphql_api/src/schema.rs`

- Añadido tipo `GpuInfo { vram_gb, model, slots_total, slots_available }`
- Añadido tipo `NodeResources` con campo `gpu: Option<GpuInfo>`
- Query `nodeResources` detecta recursos del nodo local

### T7. Detección AMD (Linux) ✅

**Archivo**: `resource_manager/src/gpu.rs`

- En `detect_gpu_platform()` Linux, después de NVIDIA, intenta AMD:
  - Lee `/sys/class/drm/card*/device/mem_info_vram_total` (bytes)
  - Lee `/sys/class/drm/card*/device/marketing_name`
- Suma VRAM de múltiples GPUs AMD

### T8. Tests de integración GPU ✅

**Archivo**: `scheduler/src/lib.rs`

- `test_gpu_job_end_to_end`: JobOffer con GPU → nodo con GPU → acepta
- `test_gpu_job_rejected_without_gpu`: JobOffer con GPU → nodo sin GPU → rechaza
- `test_gpu_job_rejected_insufficient_vram`: JobOffer con 8 GiB → nodo con 4 GiB → rechaza

### T9. Documentación y README ✅

- Actualizado README: F9 → ✅
- Añadida sección "GPU Distribuida (F9)" con ejemplos
- Documentado query GraphQL `nodeResources`

---

---

## Resumen

**Tiempo total invertido**: ~4 horas (vs ~6 horas estimadas)

**Archivos modificados**:
- `market_protocol/src/lib.rs` — JobOffer GPU fields
- `vm_runtime/src/lib.rs` — Manifest GPU field
- `vm_runtime/src/wasmtime_runner.rs` — Tests actualizados
- `resource_manager/src/gpu.rs` — GpuSlotManager + AMD detection
- `daemon/src/main.rs` — GPU wiring + slot lifecycle integration
- `scheduler/src/lib.rs` — GPU integration tests
- `graphql_api/src/schema.rs` — GpuInfo + NodeResources types
- `README.md` — F9 marked complete + GPU documentation

**Tests añadidos**: 12 nuevos tests (6 en resource_manager, 3 en scheduler, 2 en market_protocol, 1 en vm_runtime)

**CI status**: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` — todo pasa ✅

---

## Fuera de scope (post-F9)

- **Ejecución GPU real** (CUDA kernels via WASM host functions, o WebGPU) — requiere diseño de API de host functions y un WASM module que las use. Es un proyecto separado.
- **Intel GPU detection** — baja prioridad, pocos datacenters usan Intel para compute.
- **GPU metrics en WASM** — depende de ejecución GPU real.
- **Mutation `requestGpuJob`** — el schema GraphQL expone el estado GPU pero no tiene mutation para crear jobs GPU (requiere integración con el sistema de market_protocol).
