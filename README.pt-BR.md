# ledger_me_sdk

🌐 [English](README.en.md) · **Português**

`ledger_me_sdk` é um SDK em Rust para construir blockchains modulares. A
lógica de negócio é escrita em **pallets**: crates Rust compiladas para
WebAssembly e executadas pelo host num runtime isolado do
[wasmtime](https://wasmtime.dev), com a coordenação feita pelo modelo de
atores do [actix](https://actix.rs).

> **Status:** em desenvolvimento inicial. A API pública pode mudar entre
> versões. Veja o [ROADMAP](ROADMAP.pt-BR.md) para o que já está
> implementado e o que está planejado.

## Motivação

Criar uma blockchain ainda é trabalho para poucos especialistas. Esta crate
nasceu para **democratizar a criação de blockchains**: ela oferece uma
**camada modular** para que empresas e desenvolvedores montem as próprias
redes a partir de peças prontas, em vez de começar do zero.

O objetivo é que mais gente olhe com mais carinho para a tecnologia
blockchain e comece a implementar mais blockchains que agreguem valor de
verdade aos seus negócios.

## Design

- **Isolamento.** Cada pallet é um módulo WASM separado, instanciado de
  novo a cada chamada; o estado só persiste pela storage. Um trap ou um
  pânico dentro de um pallet derruba apenas aquela chamada.
- **Execução determinística.** O SDK é dono da engine do wasmtime e da
  configuração dela (medição por fuel e NaN canônico ligados; relaxed
  SIMD, threads e multi-memory desligados), então a mesma chamada produz
  o mesmo resultado em todos os nós.
- **Limites de execução.** Cada chamada leva um limite de fuel, e cada
  pallet tem um limite de memória. Passar de qualquer um deles faz a
  chamada falhar sem afetar o runtime. Veja
  [Limites de execução](#limites-de-execução).
- **Execução ordenada.** O `Runtime` processa uma chamada de nível mais
  alto por vez, e cada pallet roda na própria thread.
- **Host functions.** Funções nativas são expostas aos pallets com
  `#[host_fn]`. Rodam em velocidade nativa e custam 1 de fuel por chamada
  ao pallet que chama.
- **Macros do guest.** O `#[call]` e o `#[import]` geram a cola da ABI do
  WASM e a codificação Borsh, então as funções do pallet são escritas como
  funções Rust tipadas.
- **Tracing.** Toda chamada é um span do
  [`tracing`](https://docs.rs/tracing) com `pallet`, `function`, `elapsed`
  e `fuel_used`. O SDK só emite spans; instalar um subscriber fica a
  cargo da aplicação.

## Uso

Um pallet (o guest) é compilado para WASM separadamente. O host (o seu
nó) carrega o pallet e manda `Call`s para ele.

### 1. Pallet (guest)

`Cargo.toml`:

```toml
[package]
name = "my_pallet"
version = "0.1.0"
edition = "2021"

[workspace]

[lib]
crate-type = ["cdylib"]

[dependencies]
ledger_me_sdk_guest = "0.1"
borsh = { version = "1", default-features = false, features = ["derive"] }

[profile.release]
opt-level = "z"
lto = true
panic = "abort"
```

`src/lib.rs`:

```rust
#![no_std]

// Rust's `alloc` crate (`Vec`, `String`). The allocator itself (talc) and
// the `alloc` export the host uses come from `ledger_me_sdk_guest`.
extern crate alloc;

use alloc::vec::Vec;
use ledger_me_sdk_guest::borsh::{BorshDeserialize, BorshSerialize};
use ledger_me_sdk_guest::{call, import};

// Functions the host exposes to this pallet.
#[import(module = "crypto")]
extern "C" {
    fn sha256(data: &[u8]) -> Vec<u8>;
}

#[derive(BorshSerialize, BorshDeserialize)]
pub struct SumArgs {
    pub a: u32,
    pub b: u32,
}

// `#[call]` decodes the input and encodes the output with Borsh.
#[call]
fn sum(args: SumArgs) -> u32 {
    args.a + args.b
}

// `Vec<u8>` in and out are passed through as raw bytes; the hashing
// itself runs on the host.
#[call]
fn hash(input: Vec<u8>) -> Vec<u8> {
    sha256(&input)
}
```

Compile:

```sh
cargo build --release --target wasm32-unknown-unknown
```

### 2. Runtime (host)

`Cargo.toml`:

```toml
[dependencies]
ledger_me_sdk = "0.1"
actix-rt = "2"
anyhow = "1"
borsh = { version = "1", features = ["derive"] }
sha2 = "0.11"
```

`src/main.rs`:

```rust
use std::sync::Arc;

use ledger_me_sdk::actix::Actor;
use ledger_me_sdk::adapters::InMemoryStorage;
use ledger_me_sdk::{Call, PalletBuilder, Runtime, RuntimeError};
use borsh::BorshSerialize;
use sha2::{Digest, Sha256};

#[derive(BorshSerialize)]
struct SumArgs {
    a: u32,
    b: u32,
}

/// Offered to the pallet as `crypto::sha256`. Runs natively and costs the
/// pallet 1 fuel per call, however big the input.
#[ledger_me_sdk::host_fn]
fn sha256(data: &[u8]) -> Vec<u8> {
    Sha256::digest(data).to_vec()
}

#[actix_rt::main]
async fn main() -> anyhow::Result<()> {
    let my_pallet = PalletBuilder::new()
        .with_wasm(include_bytes!("../my_pallet.wasm"))
        .with_storage(Arc::new(InMemoryStorage::new()))
        .with_memory_bytes(32 << 20) // optional: 16 MiB by default
        .with_host_functions(|linker| {
            linker.func_wrap("crypto", "sha256", sha256)?;
            Ok(())
        })?
        .build()?
        .start();

    let runtime = Runtime::new()
        .with_pallet("my_pallet", my_pallet)
        .start();

    // A typed call, with the fuel it used.
    let args = borsh::to_vec(&SumArgs { a: 3, b: 4 })?;
    let response = runtime.send(Call::new("my_pallet", "sum", args.clone())).await??;
    let sum: u32 = borsh::from_slice(&response.data)?;
    assert_eq!(sum, 7);
    println!("sum = {sum}, fuel used = {}", response.fuel_used);

    // A call that uses a host function.
    let response = runtime.send(Call::new("my_pallet", "hash", b"hello".to_vec())).await??;
    assert_eq!(response.data.len(), 32);

    // Not enough fuel: that call fails, deterministically, and the
    // runtime keeps serving the next ones.
    let starved = Call::new("my_pallet", "sum", args).with_fuel_limit(100);
    let error = runtime.send(starved).await?.unwrap_err();
    assert!(matches!(error.downcast_ref(), Some(RuntimeError::OutOfFuel { .. })));

    Ok(())
}
```

### Rodando o exemplo do repositório

O [`examples/runtime_example`](examples/runtime_example) compila um pallet
e roda com logs estruturados, incluindo trace IDs do OpenTelemetry:

```sh
cargo run -p runtime_example
```

> **Avisos esperados:** o exemplo faz duas chamadas falharem de propósito,
> e cada uma aparece no log como `WARN call failed`. Uma roda com fuel de
> menos (`with_fuel_limit(100)`) e falha com `out of fuel`; a outra manda 3
> bytes para o `sum` em vez de um `SumArgs`, e o guest entra em pânico ao
> decodificar. Elas mostram que uma chamada que falha só afeta a si mesma:
> as chamadas logo depois continuam sendo respondidas.

Trecho da saída:

```text
DEBUG call succeeded, elapsed: 1.2ms, output_bytes: 4, events: 0, fuel_used: 2121
  in call.execution with pallet: pallet_example, function: sum
INFO  summed, a: 3, b: 4, sum: 7, fuel_used: 2121
WARN  call failed, error: out of fuel (limit: 100)
WARN  call failed, error: error while executing at wasm backtrace: ... (guest panic)
INFO  still serving calls after a guest panic
```

## Limites de execução

Todos os limites são determinísticos: todo nó chega ao mesmo resultado
para a mesma chamada.

| Limite | Padrão | Configurado com | Quando excedido |
|---|---|---|---|
| Fuel por `Call` (≈ instruções WASM executadas) | 100 milhões (~5–9 ms de CPU na máquina do benchmark) | `Call::with_fuel_limit(n)` | a chamada falha com `RuntimeError::OutOfFuel` |
| Memória do guest por pallet | 16 MiB | `PalletBuilder::with_memory_bytes(n)` | a chamada falha com `RuntimeError::OutOfMemory` |
| Pânico no guest | — | — | a chamada falha; o runtime continua atendendo |

**Host functions e fuel.** O fuel só conta instruções WASM, então uma host
function custa **1 de fuel por chamada, independentemente do trabalho que
faz**. Oferecer uma função como host function (rápida, sem medição) ou
deixar que os mantenedores dos pallets a implementem em WASM (mais lenta,
medida) é uma decisão dos **mantenedores da chain**, que passam a ser
responsáveis por limitar o custo do que expõem. Veja a
[ADR 0002](<adrs/0002 [ptBR] - Engine do SDK com fuel e limite de memoria.md>).

## Roadmap

Próximos passos, em ordem:

1. **Chamadas cross-pallet:** pallets chamando uns aos outros dentro de
   uma mesma execução, com um orçamento de fuel compartilhado e proteção
   contra reentrância.
2. **Execuções atômicas:** as gravações pendentes na storage são
   descartadas se qualquer pallet da árvore de chamadas falhar.

Detalhes no [ROADMAP](ROADMAP.pt-BR.md).

## Registros de decisão de arquitetura (ADRs)

- 📁 [adrs/](adrs/)
  - [0001 - CallExecution com uma instância por execução](<adrs/0001 [ptBR] - CallExecution com uma instancia por execucao.md>)
  - [0002 - Engine do SDK com fuel e limite de memória](<adrs/0002 [ptBR] - Engine do SDK com fuel e limite de memoria.md>)

## Licença

Licenciado sob [Apache License, Version 2.0](LICENSE-APACHE) ou
[licença MIT](LICENSE-MIT), à sua escolha.
