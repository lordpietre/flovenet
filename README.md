# Flovenet

Red descentralizada P2P para conectar redes sociales. Infraestructura Rust + libp2p + WASM + IPFS + GraphQL Gateway.

---

## Guía rápida: Instalación y uso en Ubuntu

### Requisitos

- Ubuntu 22.04 o superior (x86_64)
- Rust 1.85+ ([instalar](https://rustup.rs/))
- Node.js 20+ ([instalar](https://nodejs.org/))
- OpenSSL 3 (normalmente ya viene en Ubuntu)

### Paso 1: Clonar y compilar

```bash
git clone https://github.com/flovenet/flovenet.git
cd flovenet
cargo build --release
```

Esto genera el binario en `target/release/daemon`.

### Paso 2: Iniciar el nodo P2P (Terminal 1)

Abre una terminal y ejecuta:

```bash
./target/release/daemon daemon --port 0 --api-port 9090 --roles compute,storage
```

Verás logs como:
```
INFO Starting flovenet daemon (libp2p port: 0, api: 9090, roles: [Compute, Storage])
INFO Peer ID: 12D3KooW..., listening on /ip4/.../tcp/...
INFO Metrics/API endpoint: http://0.0.0.0:9090
```

**Déjalo corriendo.** Este es tu nodo P2P que comparte recursos.

### Paso 3: Iniciar el gateway GraphQL (Terminal 2)

Abre otra terminal en el mismo directorio:

```bash
./target/release/daemon api-gateway --port 8080
```

Verás:
```
INFO Starting flovenet API gateway on port 8080
INFO Gateway Peer ID: 12D3KooW...
```

**Déjalo corriendo.** Este es el servidor API que usa el panel web.

### Paso 4: Iniciar el panel web (Terminal 3)

Abre una tercera terminal:

```bash
cd web-dashboard
npm install
npm run dev
```

Verás:
```
  VITE v6.x.x  ready in xxx ms

  ➜  Local:   http://localhost:3000/
  ➜  Network: use --host to expose
```

**Déjalo corriendo.** Este es el servidor de desarrollo del panel web.

### Paso 5: Abrir el panel web

Abre tu navegador en:

```
http://localhost:3000
```

Serás redirigido automáticamente a la página de login.

### Paso 6: Registrarse

1. Haz clic en "Register" o ve a `http://localhost:3000/register`
2. Completa el formulario:
   - **Email**: tu email (ej: `alice@example.com`)
   - **Password**: una contraseña
   - **Display Name**: tu nombre visible
3. Haz clic en "Register"
4. Serás redirigido al login

### Paso 7: Iniciar sesión

1. Ingresa tu email y password
2. Haz clic en "Login"
3. Serás redirigido al Dashboard

### Paso 8: Usar el panel web

Una vez logueado, tienes acceso a 4 páginas:

#### Dashboard (`/`)
- Ve el estado de tu nodo local
- Muestra recursos disponibles (CPU, RAM, disco)
- Acciones rápidas para compartir recursos

#### Feed (`/feed`)
- Crea posts (publicaciones)
- Ve el timeline de posts de usuarios que sigues
- Like y comenta posts

#### Profile (`/profile`)
- Busca usuarios por nombre o Peer ID
- Sigue/deja de seguir usuarios
- Ve perfiles de otros usuarios

#### Network (`/network`)
- Descubre nodos P2P en la red
- Ve información de otros nodos (recursos, roles, reputación)

---

## Comandos CLI

El binario `daemon` tiene 5 subcomandos:

### `status` — Ver estado del nodo

```bash
./target/release/daemon status
```

Muestra CPU, RAM, disco, GPU (si hay), y uptime.

### `share` — Ver recursos que compartirías

```bash
./target/release/daemon share --role compute
```

Muestra cuántos slots disponibles tienes para el rol especificado.

Roles disponibles: `compute`, `storage`, `validation`, `ai`, `social`

### `daemon` — Iniciar nodo P2P

```bash
./target/release/daemon daemon [OPTIONS]
```

Opciones:
- `--port <PORT>`: Puerto libp2p (default: `0` = automático)
- `--api-port <PORT>`: Puerto HTTP metrics/API (default: `9090`)
- `--roles <ROLES>`: Roles del nodo separados por coma (default: `compute`)
- `--swarm-key <PATH>`: Archivo de clave para red privada (opcional)

Ejemplo con todos los roles:
```bash
./target/release/daemon daemon --port 0 --api-port 9090 --roles compute,storage,validation,ai,social
```

### `api-gateway` — Iniciar gateway GraphQL

```bash
./target/release/daemon api-gateway --port 8080
```

Expone la API GraphQL en `http://localhost:8080/graphql` con playground interactivo.

### `run` — Ejecutar WASM localmente

```bash
./target/release/daemon run --manifest _start --image feed_ranker.wasm
```

Ejecuta un módulo WASM localmente (útil para testing).

---

## GraphQL API

El gateway expone GraphQL en `http://localhost:8080/graphql`.

### Playground interactivo

Abre `http://localhost:8080/graphql` en tu navegador para usar el playground de GraphQL.

### Ejemplos de queries

**Registro:**
```graphql
mutation {
  register(email: "alice@example.com", password: "secret123", displayName: "Alice") {
    token
    profile {
      peerId
      displayName
    }
  }
}
```

**Login:**
```graphql
mutation {
  login(email: "alice@example.com", password: "secret123") {
    token
    profile {
      peerId
    }
  }
}
```

**Crear post:**
```graphql
mutation {
  createPost(content: "Hola mundo desde Flovenet!") {
    cid
    content
    timestamp
  }
}
```

**Ver feed:**
```graphql
query {
  feed(limit: 10) {
    post {
      content
      author
      timestamp
    }
    author {
      displayName
    }
  }
}
```

**Suscripción a nuevos posts (WebSocket):**
```graphql
subscription {
  newPosts {
    cid
    content
    author
  }
}
```

---

## Docker

### Opción 1: Docker Compose (3 nodos + gateway)

```bash
docker compose up --build
```

Esto inicia:
- 3 nodos P2P en puertos 9091, 9092, 9093
- 1 gateway GraphQL en puerto 8080

Accede al playground: `http://localhost:8080/graphql`

### Opción 2: Docker standalone

```bash
docker build -t flovenet:latest .
docker run --rm -p 9090:9090 -p 8080:8080 -e RUST_LOG=info flovenet:latest daemon --api-port 9090
```

---

## Red privada (PSK)

Para crear una sub-red privada con clave compartida:

```bash
# Generar clave (32 bytes)
dd if=/dev/urandom bs=32 count=1 of=swarm.key

# Iniciar nodo con la clave
./target/release/daemon daemon --swarm-key swarm.key
```

Solo nodos con la misma `swarm.key` podrán comunicarse.

---

## Stack

| Capa | Tecnología |
|------|-----------|
| Lenguaje | Rust (edition 2021) |
| Networking | libp2p (Noise + Yamux + Kademlia + Gossipsub) |
| Ejecución | Wasmtime 24 (WASI preview1) |
| Almacenamiento | LocalBackend → IpfsBackend (Kubo) → S3Backend (MinIO/AWS) → HybridBackend |
| API | async-graphql + axum + WebSocket |
| Cripto | Ed25519 + ChaCha20-Poly1305 + argon2id |
| Identidad | Ed25519 keys + keystore cifrado |
| Reputación | CRDT eventualmente consistente |
| Trust | Web of Trust (2º orden) |

## Arquitectura

```
                    App Web                     App Móvil
                       │                            │
                       └──────────┬─────────────────┘
                                  │ GraphQL (WS)
                                  ▼
                         Gateway Node
                    ┌─────────────────────┐
                    │  graphql_api        │
                    │  (async-graphql     │
                    │   + axum + WS)      │
                    ├─────────────────────┤
                    │  identity (auth)    │
                    │  storage (IPFS/S3)  │
                    │  social_protocol    │
                    └────────┬────────────┘
                             │ libp2p
                             ▼
                    ┌─────────────────────┐
                    │   Red P2P           │
                    │  ┌─────┐ ┌─────┐   │
                    │  │comp.│ │stor.│   │
                    │  └─────┘ └─────┘   │
                    │  ┌─────┐ ┌─────┐   │
                    │  │valid│ │ ai  │   │
                    │  └─────┘ └─────┘   │
                    └─────────────────────┘
```

## Workspace (16 crates funcionales + 2 de test)

```
flovenet/
├── Cargo.toml               (workspace root)
├── flovenet-core/           — core multiplataforma (JNI Android)
├── daemon/                  — proceso principal (binario)
├── cli/                     — CLI con clap
├── resource_manager/        — CPU/RAM/GPU/disco
├── vm_runtime/              — trait Runner + WasmtimeRunner
├── market_protocol/         — libp2p behaviour oferta/demanda
├── p2p_cache/               — BitSwap-lite block exchange
├── reputation_engine/       — CRDT reputación
├── ipfs_layer/              — IpfsBackend (Kubo HTTP API)
├── storage/                 — StorageBackend trait + Local + S3 + Hybrid
├── crypto/                  — primitivas criptográficas
├── identity/                — keystore + PeerId
├── scheduler/               — matching + placement + reputación
├── trust_graph/             — Web of Trust
├── social_protocol/         — Post, Profile, Follow, Feed
├── graphql_api/             — async-graphql + axum + WS
├── test_harness/            — harness de integración
├── test_reporter/           — reporter de resultados
```

## Fases de implementación

| Fase | Estado |
|------|--------|
| F0 Bootstrap | ✅ |
| F1 Networking + Discovery | ✅ |
| F2 Storage Layer | ✅ |
| F3 WASM + Scheduling MVP | ✅ |
| F4 Identidad + Cripto + Biometría | ✅ |
| F5 GraphQL API Gateway | ✅ |
| F6 Reputación | ✅ |
| F7 Trust Graph + Validación | ✅ |
| F8 Replicación + S3Backend + P2P Cache | ✅ |
| F9 GPU Distribuida | ✅ |
| F10–F14 | ⬜ |

## Tests

```bash
cargo test        # ~178 tests
cargo clippy      # 0 warnings
cargo fmt         # format
```

## Variables de entorno

| Variable | Descripción | Default |
|----------|-------------|---------|
| `RUST_LOG` | Nivel de logging (`trace`, `debug`, `info`, `warn`, `error`) | `info` |
| `FLOVENET_DATA_DIR` | Directorio de datos | `~/.local/share/flovenet` |
| `FLOVENET_CACHE_DIR` | Directorio de caché | `~/.cache/flovenet` |
| `FLOVENET_PLATFORM` | Forzar plataforma (`linux`, `windows`, `android`) | auto-detect |
| `FLOVENET_GPU_VRAM_GB` | VRAM GPU en GiB (override) | auto-detect |
| `FLOVENET_GPU_MODEL` | Modelo GPU (override) | auto-detect |

Ejemplo con GPU manual:
```bash
FLOVENET_GPU_VRAM_GB=24 FLOVENET_GPU_MODEL="RTX 4090" ./target/release/daemon share --role ai
```

## GPU Distribuida (F9)

Flovenet soporta detección y asignación de recursos GPU para trabajos de IA.

### Detección automática

- **Linux NVIDIA**: Lee `/proc/driver/nvidia/gpus/*/information`
- **Linux AMD**: Lee `/sys/class/drm/card*/device/mem_info_vram_total`
- **Windows**: Usa `nvidia-smi` o `wmic` como fallback

### Slots GPU

La VRAM se divide en slots de 8, 4 o 2 GiB usando bin-packing greedy:
- 24 GiB → 3 slots de 8 GiB
- 14 GiB → 1 slot de 8 + 1 de 4 + 1 de 2 GiB
- 3 GiB → 1 slot de 2 GiB

### Consultar estado GPU via GraphQL

```graphql
query {
  nodeResources {
    cpuCores
    ramTotalGb
    gpu {
      vramGb
      model
      slotsTotal
      slotsAvailable
    }
  }
}
```

### Ciclo de vida de slots GPU

Cuando un job con `gpu_required: true` llega al daemon:
1. El scheduler verifica que el nodo tenga VRAM suficiente
2. El `GpuSlotManager` asigna slots específicos (marca como `available: false`)
3. El job se ejecuta
4. Al terminar (éxito o fracaso), los slots se liberan automáticamente

### Limitaciones actuales

- Solo detección NVIDIA (Linux/Windows) y AMD (Linux)
- Sin ejecución GPU real (CUDA/OpenCL/Vulkan) — solo accounting
- Sin passthrough GPU a WASM modules
