# TODO: o que ainda precisamos resolver

🌐 [English](TODO.md) · **Português**

Problemas e decisões em aberto. O que já está decidido e só falta
implementar fica no [ROADMAP.pt-BR.md](ROADMAP.pt-BR.md).

Vocabulário: **serial** = o ator processa uma mensagem por vez, do começo ao
fim. **Atômico** = tudo ou nada na storage. Não misturar os dois.

Marcador: **🤔 indeciso** = ainda não sei o que vou fazer; são só ideias,
nada decidido.

## Estado e consistência

- [ ] **Rollback:** hoje não existe. Se um pallet falha no meio da execução,
  as gravações feitas até ali ficam. Precisa decidir:
  - o formato das alterações pendentes;
  - como elas conversam com `Storage::commit` (hoje não faz nada; ver
    `src/adapters/sqlite_storage.rs`);
  - se a leitura dentro da transação enxerga as próprias gravações ainda
    pendentes.
- [ ] **Ordem global de longo prazo:** hoje o `Runtime` é serial e garante a
  ordem. Quando existir o ator de execução de bloco, quem deve ser o dono
  dessa garantia, ele ou o `Runtime`?
- [ ] **Reentrância:** o plano é recusar todo ciclo (A→B→A). Precisa
  confirmar se algum caso legítimo (ex.: um pallet chamando a si mesmo)
  deveria ser permitido.

## Tracing

- [ ] **ID de trace nunca entra no estado, em eventos nem no consenso.** Ele
  é aleatório e quebraria o determinismo entre nós. Como garantir isso além
  de documentar?
- [ ] **Correlação por `tx_hash`:** o hash da transação como campo do span
  (é igual em todos os nós). O conceito de transação ainda não existe no SDK.
- [x] **Onde o `CallContext` mora:** decidido, campo público `call.context`.
  Quem constrói `Call { .. }` direto precisa preencher
  (`CallContext::from_current_span()`); o `Call::new` já faz isso.

## Robustez

- [ ] **Pânico na thread de um pallet** (ex.: pânico Rust numa host
  function): o `SyncArbiter` não reinicia, e chamadas seguintes voltam
  `MailboxError`. Queremos supervisão? Traps do WASM, inclusive pânico
  dentro do guest (o panic handler dele dá trap com `unreachable`), já
  chegam como `Err` e não matam a thread.
- [x] **Guest preso em laço infinito:** decidido na
  [ADR 0002](<adrs/0002 [ptBR] - Engine do SDK com fuel e limite de memoria.md>)
  (fuel, `Engine` do SDK, limite de memória por pallet) e implementado na
  fase 3 do ROADMAP.
- [ ] **Profundidade da pilha não é determinística:** recursão profunda dá
  trap (não trava), mas o limite do wasmtime é em bytes nativos, então um
  nó pode dar trap onde outro não dá. O Substrate resolveu instrumentando
  o `.wasm` para contar a profundidade. Vale a pena?
- [ ] **Uma host function custa 1 de fuel por chamada, faça o que fizer**
  (inclusive um `sha256` de 100 MB). Oferecer host functions é decisão dos
  mantenedores da chain (ADR 0002); em aberto: o SDK deve ajudar a cobrar
  (o `Caller` consegue ler e mudar o fuel), ex.: o `#[host_fn]` cobrando
  por byte? Isso já é "gas" de verdade.
- [ ] **"Pallet nunca manda `Call` para o `Runtime`":** hoje isso só está na
  documentação. Dá para impedir pelo tipo (o pallet não ter acesso ao
  `Addr<Runtime>`)?

## Performance

Números e contexto na [ADR 0001](<adrs/0001 [ptBR] - CallExecution com uma instancia por execucao.md>).
Hoje um `Call` completo custa ~230–270 µs, e com o `Runtime` serial isso dá
uns **4 mil `Call`s/s**. Criar o `CallExecution` por execução custa ~0,2%
disso e não é gargalo.

- [ ] 🤔 **indeciso: salto de thread em todo `Call`** (~60 µs no WSL2).
  Ideia: mandar um bloco inteiro de `Call`s numa mensagem só, para pagar o
  salto uma vez por bloco.
- [ ] 🤔 **indeciso: instanciar o WASM em toda chamada** (~56–82 µs). Ideia:
  alocador pooling do wasmtime (`PoolingAllocationConfig`). O `InstancePre`
  sozinho quase não mudou.
- [ ] 🤔 **indeciso: ~100 µs sem explicação.** O caminho pelo ator do pallet
  custa mais do que salto de thread e WASM somados. Investigar antes de
  otimizar qualquer outra coisa.
- [ ] 🤔 **indeciso: chamadas cross-pallet custam dois saltos de thread a mais**
  (A → `CallExecution` → B → volta). Pesa muito mais do que criar o ator. Não
  bloqueia o `CallExecution`, mas precisa ser encarado quando a chamada entre
  pallets entrar.
- [ ] 🤔 **indeciso: execução paralela** de transações que não mexem nos
  mesmos dados. Bem mais para frente.
- [ ] Medir de novo em **Linux nativo** antes de otimizar: no WSL2 os números
  variaram bastante entre rodadas.
