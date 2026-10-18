# Roadmap

🌐 [English](ROADMAP.md) · **Português**

O que vamos implementar, na ordem. Regra do projeto: **"Tudo no Actix, nada
contra o Actix, nada fora do Actix."** Decisões ainda em aberto ficam no
[TODO.pt-BR.md](TODO.pt-BR.md).

## ✅ 1. Tracing (feito)

- O SDK depende só de `tracing` (a fachada). Ele emite spans e eventos, mas
  **nunca instala subscriber**. Isso é decisão de quem monta o nó.
- `CallContext` viaja dentro de todo `Call` (campo público `call.context`).
  Por enquanto tem só o `parent_span`, que o `Call::new` preenche com
  `Span::current()`, então quem usa o SDK não muda nada.
- Spans, os dois com os campos `pallet` e `function`:
  - `call.execution` no `CallExecution` (ver fase 2): raiz de cada chamada;
  - `pallet.call` no `Pallet`: `span.enter()` na thread do `SyncArbiter`.
  - O span `runtime.route` planejado saiu: o `call.execution` já é a raiz de
    cada chamada, e um span no `Runtime` só repetiria ele.
- Toda chamada termina com um evento: `call succeeded` (`DEBUG`: `elapsed`,
  `output_bytes`, `events`) ou `call failed` (`WARN`: `elapsed`, `error`).
- `runtime_example` loga para desenvolvimento com o formato `pretty()` do
  `tracing-subscriber` (árvore de spans, arquivo e linha em cada evento). O
  `RUST_LOG` substitui o filtro padrão `info,ledger_me_sdk=debug`.
- `runtime_example` também instala uma camada `tracing-opentelemetry` e
  mostra o `trace_id` de cada evento (um por árvore de chamada) no terminal
  (`src/logging.rs`). O SDK não mudou: trace ID é trabalho do subscriber.
  Nada é exportado ainda.
- Teste: `tests/tracing_spans.rs` confere a árvore de spans
  (quem chamou → `call.execution` → `pallet.call`).

## ✅ 2. `CallExecution`: um ator por execução de chamada (feito)

Decidido na [ADR 0001](<adrs/0001 [ptBR] - CallExecution com uma instancia por execucao.md>):
uma instância nova por `Call` de nível mais alto, e não uma fixa
reaproveitada.

- O `Runtime` continua **serial**: um `Call` de nível mais alto por vez. Para
  cada `Call` ele cria um `CallExecution`, que coordena toda a árvore de
  chamadas daquele `Call` e morre no final.
- Nesta fase o `CallExecution` só guarda o span raiz e junta os eventos.
  Ele para sozinho quando o `Runtime` solta o `Addr` dele.
- Os handlers do `CallExecution` usam `ResponseActFuture` (não seriais), para a
  mailbox continuar livre enquanto ele espera um pallet.

## ✅ 3. `Engine` do SDK e limites de execução (feito)

Decidido na [ADR 0002](<adrs/0002 [ptBR] - Engine do SDK com fuel e limite de memoria.md>):
um guest não pode conseguir travar nem derrubar o nó, e todo limite
precisa ser determinístico.

- O SDK cria a `Engine` (uma por processo) com fuel, NaN canônico, sem
  relaxed SIMD, sem threads e sem multi-memory. O `PalletBuilder` recebe os bytes do WASM
  (`with_wasm`) no lugar de `with_engine`/`with_module`, e o `build()`
  devolve `Result`.
- O limite de fuel viaja no `Call` (`call.fuel_limit`): o `Call::new` põe
  100 milhões, e o `with_fuel_limit(n)` muda. O `Pallet` abastece o
  `Store` antes de instanciar. Quando acaba, volta
  `RuntimeError::OutOfFuel`, só daquele `Call`. O `CallResponse` ganha
  `fuel_used`, que o `call succeeded` também loga.
- Limite de memória por pallet: `with_memory_bytes(n)` opcional, padrão de
  16 MiB, via o `ResourceLimiter` do próprio SDK. O
  `build()` falha se a memória inicial do módulo já passar dele. Crescer
  além do limite faz a chamada falhar com `RuntimeError::OutOfMemory` (um
  trap, nunca -1), e o `alloc` do guest SDK dá trap em ponteiro
  nulo. Tabelas limitadas a 100 mil itens
  (`RuntimeError::TableLimitExceeded`).
- O rustdoc do `Pallet` avisa que mandar direto para o `Addr<Pallet>` pula
  o `CallExecution`.
- O rustdoc do `#[host_fn]` e do `with_host_functions` diz que **uma host
  function custa 1 de fuel por chamada**, faça o que fizer, e que
  disponibilizá-la (rápida, sem medição) ou deixar para os pallets em WASM
  (mais lento, medido) é decisão dos mantenedores da chain.
- O `RuntimeError::OutOfFuel` leva o `fuel_limit` que acabou, então o log
  diz qual limite foi atingido.
- O `alloc` do guest SDK também parou de pedir 0 bytes ao alocador
  (comportamento indefinido; um `Call` com input vazio fazia isso).
- O `runtime_example` mostra o `fuel_used` e um `Call` falhando com
  `OutOfFuel` (`with_fuel_limit(100)`).
- Testes (`tests/execution_limits.rs`): um laço infinito em WAT, um
  `memory.grow` e um `table.grow` além dos limites voltam `Err`, e o
  `Call` seguinte continua respondendo; o `build()` recusa um módulo com
  memória inicial acima do limite e um com duas memórias.

## 4. Chamadas cross-pallet através do `CallExecution`

- O `CallContext` ganha o `Addr<CallExecution>` (veio da fase 2: nada usa
  ele antes de existirem chamadas cross-pallet).
- Guest: `#[import(pallet = "nome")]` gera um módulo WASM com prefixo
  reservado (ex.: `pallet:nome`). No `build()`, todo import com esse prefixo
  é ligado a um encaminhador que manda o `Call` para o `CallExecution`.
- O `CallExecution` resolve o destino pela tabela de rotas que o `Runtime`
  empresta (um `Arc<HashMap<nome, Addr<Pallet>>>`).
- `call_chain` (pilha de pallets ativos): o `CallExecution` recusa com erro
  um destino que já está na pilha, em vez de travar (proteção contra
  reentrância).
- Um pallet **nunca** manda `Call` para o `Addr<Runtime>`, porque ele está
  esperando esse mesmo pallet e travaria (deadlock).
- Opcional: o `Runtime` valida no `started()` se todo pallet importado está
  registrado.
- O limite de fuel do `Call` vira o orçamento da árvore de chamadas
  inteira: o `CallExecution` passa para B o que sobrou de A e desconta o
  que B gastou (ADR 0002).

## 5. Alterações pendentes e rollback (atomicidade)

- O `CallExecution` é dono das alterações pendentes da transação
  (`Arc<Mutex<...>>` emprestado aos pallets pelo contexto): grava tudo se der
  certo, descarta tudo se qualquer nível falhar.
- Não há disputa pelo lock: dentro de uma transação só um pallet roda por
  vez.

## Depois (quando houver necessidade real)

- **Tracing distribuído:** na borda de rede ou RPC, propagar no padrão W3C
  `traceparent` com `tracing-opentelemetry` e exportar via OTLP (Jaeger,
  Tempo). Só faz sentido quando existir rede. Os trace IDs em si já existem
  no `runtime_example`. Não precisa mudar o SDK:
  - **Receber** um trace de fora: `TraceContextPropagator::new().extract(&headers)`,
    depois `span.set_parent(upstream)` num span nosso, **antes** de rodar
    qualquer coisa dentro dele, e criar o `Call` dentro desse span. Testado
    na mão: a chamada saiu no log com o `trace_id` de fora.
  - **Repassar** adiante: `span.context()` + `TraceContextPropagator::inject_context(...)`
    geram o `traceparent` de saída.
  - O `traceparent` leva o `trace_id` **e** o `span_id` de quem chamou. Com
    só um trace ID solto, dá para montar o contexto na mão
    (`SpanContext::new`), mas perde-se o elo com o span de quem chamou.
  - O `traceparent` viaja no envelope/metadados da mensagem, nunca em
    conteúdo assinado ou hasheado (determinismo, ver TODO).
- **Log no guest:** host function `log::emit(level, msg)` e um macro `log!`
  no `ledger_me_sdk_guest`, virando `tracing::event!` dentro de `pallet.call`.
- **Eventos como mensagens:** broadcast para `Recipient<PalletEvent>` quando
  existir quem consuma (RPC, indexer).
- **Atores de nó:** execução de bloco, mempool e consenso, conversando com o
  `Addr<Runtime>`.
