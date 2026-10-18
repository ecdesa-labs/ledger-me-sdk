# CLAUDE.md

Guia para o Claude trabalhar neste repositório. O que está aqui são as
regras de **processo** (documentação, ADRs, commits, idioma) e as
**invariantes** que não dá para deduzir lendo um arquivo só. Detalhes de
código ficam na documentação do próprio código.

## O projeto em uma frase

`ledger_me_sdk` é um SDK em Rust para criar blockchains de forma modular:
a lógica de negócio fica em **pallets** (Rust compilado para WASM, rodando
no `wasmtime`), e tudo é orquestrado com o modelo de atores do **actix**.
O porquê está em [README.pt-BR.md](README.pt-BR.md).

## Princípios (valem para toda decisão)

- **"Tudo no Actix, nada contra o Actix, nada fora do Actix."** Coordenação
  e concorrência vão por atores e mensagens. Recursos (storage, engine)
  pertencem a atores e são emprestados; não viram atores sem motivo.
- **Fácil de manter:** o dono do projeto tem TDAH. Nomes descritivos (sem
  exagerar no tamanho), pouca indireção, nada de abstração com um só uso.
- **Ergonomia para quem usa o SDK:** a API pública deve ser óbvia. Prefira
  que o SDK faça sozinho o que for repetitivo (ex.: `Call::new` já captura
  o span atual).
- **Converse em português (pt-BR)** com o usuário.

## Vocabulário

- **serial**: o ator processa uma mensagem por vez, do começo ao fim (é o
  comportamento do `AtomicResponse` do actix, usado no `Runtime`).
- **cross-pallet**: um pallet chamando outro no meio de uma execução
  (A → `CallExecution` → B). Não usar "chamada aninhada" / "nested call".
- **atômico**: tudo ou nada na storage (rollback). Nunca chamar o `Runtime`
  de "atômico".
- **🤔 indeciso / undecided** (no TODO): o usuário ainda não decidiu; são
  só ideias.

## Onde cada coisa mora

| Arquivo | Para quê |
|---|---|
| `README.md` | Só roteia para as versões de idioma. Sem conteúdo. |
| `README.en.md` / `README.pt-BR.md` | Por que a crate existe, exemplo de uso (guest + host) e árvore de ADRs. |
| `ROADMAP.md` / `ROADMAP.pt-BR.md` | O que vamos implementar, em ordem. Fases feitas ficam marcadas com ✅. |
| `TODO.md` / `TODO.pt-BR.md` | Problemas e decisões **em aberto**. |
| `adrs/` | Decisões de arquitetura, com contexto e medições. Tabela abaixo. |

**Leia o `ROADMAP` e o `TODO` antes de planejar ou implementar qualquer
coisa.** Quando uma fase terminar ou uma decisão for tomada, atualize os
dois (nos dois idiomas).

## Documentação: checklist depois de toda mudança de código

Sempre que mudar ou acrescentar algo no código, antes de dizer que terminou,
verifique se precisa atualizar:

1. **Rustdoc** dos itens mexidos e a doc do crate (`src/lib.rs`). Links para
   itens privados quebram o build de doc.
2. **README** (`en` e `pt-BR`): o exemplo de uso ainda compila e roda com a
   API nova? Se a API pública mudou, teste o exemplo de verdade (crate
   temporária na pasta de scratch, fora do repositório).
3. **ROADMAP / TODO** (os dois idiomas): marcar o que foi feito, registrar
   o que mudou em relação ao plano, fechar itens resolvidos.
4. **ADR**: a mudança tomou uma decisão de arquitetura? Veja a seção abaixo.

Diga ao usuário o que foi atualizado e o que foi conferido e não precisou
mudar.

## Bilíngue: sempre inglês e português

Toda documentação para humanos existe nos dois idiomas, e as duas versões
andam juntas em toda edição.

- Inglês é o padrão (`ROADMAP.md`), português tem o sufixo
  (`ROADMAP.pt-BR.md`). Exceção: `README.md` é só o roteador, com
  `README.en.md` e `README.pt-BR.md`.
- ADRs: `adrs/NNNN [en] - Title.md` e `adrs/NNNN [ptBR] - Titulo.md`, com o
  título **sem acento** no nome do arquivo.
- Todo arquivo começa com o seletor de idioma
  (`🌐 **English** · [Português](...)`) e só linka arquivos do **mesmo**
  idioma.
- Links para nomes com espaço vão entre `<...>`:
  `[ADR 0001](<adrs/0001 [en] - ....md>)`.
- Rustdoc e comentários de código: só inglês.
- Depois de mexer em links, confira se todos apontam para arquivos que
  existem.

## ADRs

**Proponha uma ADR** (pergunte antes de escrever) quando uma decisão:

- mudar a arquitetura ou uma invariante (ver abaixo);
- escolher entre alternativas reais, descartando alguma;
- for baseada em medição (benchmark), para os números não se perderem.

Formato: Status, Data, commit da época, Contexto, Medição (se houver),
Decisão, Consequências. **Sem código longo** (ex.: o código do benchmark não
entra; basta descrever o método e o ambiente). Toda ADR nova entra:

1. nas duas versões de idioma, em `adrs/`;
2. na árvore de ADRs do `README.en.md` e do `README.pt-BR.md`;
3. na tabela abaixo, com um resumo que dê para pesquisar sem abrir o
   arquivo.

### Tabela de ADRs

| # | Decisão | Resumo | Arquivos |
|---|---|---|---|
| 0001 | `CallExecution` com uma instância por execução | O `Runtime` (serial) cria um ator `CallExecution` novo para cada `Call` de nível mais alto, e ele morre no fim; chamadas cross-pallet reusam a mesma instância. Benchmark (WSL2): criar o ator custa ~1 µs contra ~0,6 µs de reusar, ≈0,2% de um `Call` (~230–270 µs, ≈4 mil/s). A instância fixa foi descartada para não vazar estado entre transações. O custo real está no salto de thread (~60 µs), na instanciação do WASM (~56–82 µs) e em ~100 µs sem explicação (ver TODO, Performance). | [en](<adrs/0001 [en] - CallExecution with one instance per execution.md>) · [pt-BR](<adrs/0001 [ptBR] - CallExecution com uma instancia por execucao.md>) |
| 0002 | `Engine` do SDK com fuel e limite de memória | Guest não pode travar nem derrubar o nó, e o limite precisa ser determinístico. O SDK cria a `Engine` (fuel, NaN canônico, sem relaxed SIMD/threads/multi-memory); o `PalletBuilder` recebe os bytes (`with_wasm`). Fuel viaja no `Call` (`call.fuel_limit`, padrão 100M ≈ 5–9 ms no WSL2), acabar é `RuntimeError::OutOfFuel`, `CallResponse.fuel_used` volta o gasto, e vira orçamento da árvore no cross-pallet; memória por pallet (`with_memory_bytes`, padrão 16 MiB) com trap ao crescer além do limite (senão o `alloc` do guest devolve 0 e o host escreve por cima da pilha); tabelas limitadas a 100 mil itens. Atualização: erros com tipo `OutOfMemory`/`TableLimitExceeded` via `ResourceLimiter` próprio do SDK, porque o `StoreLimits` do wasmtime só reporta texto. Epoch descartado (não determinístico). Benchmark: sem custo mensurável nos `Call`s atuais, ~+10–25% na execução WASM de código real, até ~+50–65% em laço minúsculo. **Host function custa 1 de fuel por chamada, faça o que fizer** (medido; `#[host_fn]` soma ~217 fixos do `alloc`): disponibilizar host functions (rápidas, sem medição) ou deixar os pallets implementarem em WASM (medido) é decisão dos mantenedores da chain. Pilha ainda não determinística (TODO). | [en](<adrs/0002 [en] - SDK-owned Engine with fuel and memory limits.md>) · [pt-BR](<adrs/0002 [ptBR] - Engine do SDK com fuel e limite de memoria.md>) |

## Invariantes da arquitetura (não quebrar sem ADR)

- **`Pallet` é `SyncActor` em `SyncArbiter::start(1, ..)`** (`Pallet::start`).
  WASM é trabalho bloqueante e nunca roda no event loop. Uma thread só, de
  propósito: garante a ordem das chamadas (determinismo).
- **`Runtime` é serial** (`AtomicResponse`): um `Call` de nível mais alto
  por vez.
- **Cada `Call` roda num `CallExecution` novo** (ADR 0001), que abre o span
  `call.execution` e junta os eventos.
- **Um pallet nunca manda `Call` para o `Addr<Runtime>`** no meio de uma
  execução: o `Runtime` está esperando esse mesmo pallet (deadlock).
- **O SDK só emite spans pelo `tracing`, nunca instala subscriber.** Trace
  ID é trabalho do subscriber (ver `examples/runtime_example/src/logging.rs`).
- **Trace ID nunca entra no estado, em eventos nem em conteúdo assinado ou
  hasheado**: quebraria o determinismo entre nós.
- **O SDK é dono da `Engine`** (`src/engine.rs`, ADR 0002): configuração
  determinística (fuel, NaN canônico, sem relaxed SIMD/threads/
  multi-memory). Nunca aceitar `Engine` nem `Module` de fora.
- **Todo `Store` recebe fuel e limites antes de instanciar**
  (`Pallet::run`): a função `start` do módulo também gasta fuel.
- **Crescer memória ou tabela além do limite dá trap, nunca -1**
  (o `ResourceLimiter` do SDK, em `src/pallet/pallet_context.rs`, com erro com tipo
  `OutOfMemory`/`TableLimitExceeded`): senão o `alloc` do guest devolve 0
  e o host escreve por cima da pilha do guest.
- **Todo limite de execução é determinístico** (fuel, bytes de memória,
  itens de tabela). Nada baseado em tempo real.

## Armadilhas conhecidas

- `ledger_me_sdk_guest` e os pallets de exemplo ficam **fora** do workspace
  raiz (cada um tem seu próprio `[workspace]`). Dentro do workspace, a
  unificação de features do Cargo liga o `std` do `borsh` e quebra o
  `#![no_std]` do guest (panic handler duplicado, E0152).
- Um guest que deriva `BorshSerialize`/`BorshDeserialize` precisa depender
  do `borsh` direto no próprio `Cargo.toml`; o re-export do
  `ledger_me_sdk_guest` não basta para o derive.
- O crate de macros é edition 2024: o código gerado precisa das formas
  `unsafe extern "C"` / `#[unsafe(no_mangle)]`, mesmo que o guest seja
  edition 2021.

## Verificação antes de dizer que terminou

```sh
cargo test                                   # unit, integração (tests/) e doctests
cargo clippy --workspace --all-targets       # único aviso conhecido: Default do PalletBuilder
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
cargo run -p runtime_example                 # exemplo de ponta a ponta, com logs
```

Números de performance medidos no WSL2 variam bastante entre rodadas; não
tire conclusão de uma rodada só.

## Git

- Commit e push **só quando o usuário pedir**, e só o que foi pedido (commit
  não implica push).
- Branch de trabalho: `main` (remote `origin` =
  `git@github.com:ecdesa-labs/ledger-me-sdk.git`).
- A identidade do git (local, deste repositório) é `ecdesa-labs` /
  `github@ecdesa.com.br`, por escolha do usuário. Não mude.
- Mensagens de commit em inglês, no estilo `feat:` / `docs:` / `refactor:`.
