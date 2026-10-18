# ADR 0001: `CallExecution` com uma instância por execução

🌐 [English](<0001 [en] - CallExecution with one instance per execution.md>) · **Português**

- **Status:** Aceita
- **Data:** 2026-09-22
- **Código na época:** commit `f13ae7f`

## Contexto

O [ROADMAP](../ROADMAP.pt-BR.md) prevê um ator `CallExecution` para coordenar a
árvore de chamadas de cada `Call` de nível mais alto: span raiz, eventos,
chamada cross-pallet com `call_chain` e, depois, rollback.

A dúvida era de custo. Uma blockchain pode receber milhares de execuções por
segundo, e havia duas formas de ter esse ator:

1. **Uma instância por execução:** para cada `Call` de nível mais alto, o
   `Runtime` cria um `CallExecution` novo (`.start()`, `Addr` novo). As
   chamadas cross-pallet reusam essa mesma instância, e ela morre no fim.
2. **Uma instância fixa:** um único `CallExecution` criado junto com o
   `Runtime` e reaproveitado, com o estado limpo entre uma execução e outra.

Como o `Runtime` é **serial** (um `Call` de nível mais alto por vez), nas duas
opções só existe uma execução ativa por vez. A diferença entre elas é só o
custo de criar o ator e o risco de sobrar estado de uma execução para a
próxima.

## Medição

Benchmark em release (crate avulsa, fora do repositório), 200 mil repetições para operações
de ator e 20 mil para `Call`s, depois de um aquecimento. Ambiente: WSL2
(kernel `6.18.33.2-microsoft-standard-WSL2`), 16 núcleos, `rustc 1.98.1`.
Os números variaram bastante entre rodadas; abaixo está a faixa observada.

| Operação | Custo por operação |
|---|---|
| Criar ator novo (`Context`), mandar mensagem e esperar | ~0,95–1,14 µs |
| Mandar mensagem para um ator que já existe | ~0,60 µs |
| Salto para a thread do `SyncArbiter(1)`, sem WASM | ~60 µs |
| Só o WASM: criar o `Store`, instanciar o módulo, rodar o `sum` | ~56–82 µs |
| Só o WASM, com `InstancePre` | ~57–67 µs |
| `Call` direto no `Addr<Pallet>` | ~234 µs |
| **`Call` completo** (`Runtime` → `Pallet`) | **~227–275 µs** (≈ 3,6–4,4 mil/s) |

## Decisão

**Uma instância de `CallExecution` por execução de `Call` de nível mais
alto.** Chamadas cross-pallet usam a instância da execução em andamento.

- Criar o ator custa uns ~0,4–0,5 µs a mais do que reusar, cerca de **0,2%**
  do custo de um `Call`. Não é gargalo.
- Uma instância fixa economizaria só isso, mas exigiria limpar o estado entre
  execuções. Numa blockchain, estado que sobra de uma transação para outra é
  um bug grave. Com uma instância por execução, o estado morre junto com ela
  e esse tipo de bug não existe.

## Consequências

- A vazão hoje é limitada pela latência de um `Call` (o `Runtime` é serial):
  uns **4 mil `Call`s/s**. O `CallExecution` não muda esse número de forma
  relevante.
- O custo real está em outro lugar: o salto de thread e a instanciação do
  WASM em toda chamada, mais uns ~100 µs que ainda não foram explicados.
  Isso está registrado em [TODO.pt-BR.md, seção Performance](../TODO.pt-BR.md#performance).
- Cada chamada cross-pallet vai custar **dois saltos de thread a mais**
  (A → `CallExecution` → B → volta). Isso pesa bem mais do que criar o ator e
  também está no TODO.
- Os números são do WSL2, onde acordar uma thread costuma ser mais lento.
  Medir de novo em Linux nativo antes de otimizar qualquer coisa.
