# ADR 0002: `Engine` do SDK com fuel e limite de memória

🌐 [English](<0002 [en] - SDK-owned Engine with fuel and memory limits.md>) · **Português**

- **Status:** Aceita
- **Data:** 2026-09-23
- **Código na época:** commit `5325994`

## Contexto

Um pallet cujo WASM nunca termina prende a thread do `Pallet` para sempre.
Como o `Runtime` é **serial**, prende também todos os `Call`s seguintes,
de todos os pallets: a chain inteira para.

Até o commit `5325994`, um pânico dentro do guest fazia exatamente isso: o
panic handler do guest era `loop {}`. Esse commit trocou por um trap
(`unreachable`), e agora um pânico só derruba o próprio `Call`. Mas o
guest ainda pode entrar em laço infinito por conta própria (bug ou
má-fé), e nada o interrompe. Ele também pode crescer a memória até o
limite de 4 GiB do wasm32 em cada `Call` e derrubar o nó.

Qualquer limite precisa ser **determinístico**: todos os nós precisam
chegar ao mesmo resultado para o mesmo `Call`, senão os estados divergem.

O wasmtime oferece dois jeitos de interromper um guest:

1. **Fuel:** conta as instruções WASM executadas (a maioria custa 1) e dá
   trap quando acaba. Determinístico. Só o WASM é contado: **chamar uma
   host function custa 1 de fuel** (a instrução `call`), não importa
   quanto trabalho o host faça.
2. **Epoch interruption:** uma thread externa avança um relógio, e o guest
   dá trap depois de um prazo. Mais barato, mas baseado em tempo real: um
   nó lento pode dar trap onde um nó rápido não dá.

O fuel é ligado na `Engine` (`Config::consume_fuel`), e hoje quem cria a
`Engine` e compila o `Module` é quem usa o SDK. A `Engine` do wasmtime não
tem getters públicos, então o SDK não consegue saber se uma `Engine`
recebida tem fuel, nem as outras configurações que afetam o determinismo
(NaN canônico, relaxed SIMD, threads).

## Medição

Benchmark em release (crate avulsa, fora do repositório), wasmtime 48.0.2.
Mede só a parte WASM de um `Call` (o que o `Pallet::handle` faz: `Store`
novo, instanciar, `alloc`, escrever o input, chamar, ler o output), porque
o fuel não mexe no caminho dos atores. Aquecimento de 10% das repetições.
Duas configurações de `Engine`: a padrão e a do SDK (fuel, NaN canônico,
sem relaxed SIMD, sem threads, `StoreLimits` de 16 MiB). Cinco rodadas de
cada, mais três rodadas com **só** o fuel ligado, para isolar o custo dele.
Ambiente: WSL2 (kernel `6.18.33.2-microsoft-standard-WSL2`), 16 núcleos,
`rustc 1.98.1`.

| Caso | Padrão | Config do SDK | Diferença |
|---|---|---|---|
| `pallet_example::sum` (20 mil repetições) | ~57–74 µs | ~58–79 µs | nenhuma mensurável |
| `pallet_example::hash`, sha256 numa host function (20 mil) | ~59–79 µs | ~61–72 µs | nenhuma mensurável |
| Guest Rust, sha256 **dentro do WASM**, encadeado 1000× (500) | ~522–555 µs | ~589–657 µs | ~+10–25% |
| Guest Rust, sha256 **dentro do WASM** de 64 KiB (2000) | ~441–581 µs | ~474–532 µs | ~0–15% |
| Laço WAT, 14 instruções × 1 milhão de voltas (300) | ~606–833 µs | ~917–996 µs | ~+20–60% (pior caso) |
| Laço WAT com load/store × 1 milhão de voltas (300) | ~634–849 µs | ~981–1.101 µs | ~+30–65% (pior caso) |

- Só com o fuel ligado, os laços custaram o mesmo que a config completa do
  SDK: **o overhead é todo do fuel**. O NaN canônico não custa nada sem
  float, e o limitador de memória só age no `memory.grow`.
- O fuel gasto foi **idêntico em todas as rodadas**: `sum` 2.106; `hash`
  2.740; sha256 dentro do WASM encadeado 1000× 7.425.605; de 64 KiB
  5.640.685.
- Com o fuel ligado, esta máquina executa uns **11–19 milhões de fuel por
  ms** (11–12 no sha256 real, 15–19 nos laços).
- Um guest Rust começa com 17 páginas de memória (1,06 MiB: 1 MiB de
  pilha mais os dados).
- **Chamadas de host function**, medidas à parte (1.000 chamadas num laço
  WAT, menos o laço vazio): **exatamente 1 de fuel por chamada**, tanto
  para uma host function que não faz nada quanto para uma que calcula 10
  mil sha256. Um `#[host_fn]` soma um valor **fixo** de ~217: o `alloc` do
  guest que ele chama para devolver o resultado. Isso também não depende
  do trabalho feito.

## Decisão

**O SDK é dono da `Engine`, com configuração determinística e fuel. O
limite de fuel viaja no `Call`, e cada pallet tem um limite de memória.**

1. **`Engine` do SDK.** Uma por processo, criada pelo SDK com fuel ligado,
   NaN canônico ligado, relaxed SIMD desligado, threads desligadas e
   multi-memory desligado (ver item 3). O
   `PalletBuilder` recebe os bytes do WASM e compila o `Module` por
   dentro. `with_engine`/`with_module` saem da API, e o `build()` passa a
   devolver `Result` (compilar pode falhar).
2. **O fuel vem do `Call`.** Um campo público `call.fuel_limit`, ao lado
   de `pallet`, `function` e `data`: o limite faz parte do pedido, como o
   gas limit de uma transação. O `Call::new` põe um padrão de **100
   milhões** (~5–9 ms de CPU na máquina acima, ~47 mil vezes um `sum`), e
   o `with_fuel_limit(n)` muda. O `Pallet` abastece o `Store` **antes** de
   instanciar (a função `start` de um módulo também gasta fuel). Quando
   acaba, é trap, e o SDK devolve como `RuntimeError::OutOfFuel`, um `Err`
   só daquele `Call`. O `CallResponse` ganha `fuel_used`, que o evento
   `call succeeded` também loga. Depois
   o limite vai vir da transação e, com chamadas cross-pallet, o
   `CallExecution` vai usá-lo como orçamento da árvore de chamadas inteira.
3. **Limite de memória por pallet.** Um `with_memory_bytes(n)` opcional no
   builder, com padrão de **16 MiB** (~15× a memória inicial de um guest
   Rust), aplicado no `Store` de cada `Call` (por `StoreLimits` no começo;
   ver a [atualização](#atualização-2026-09-23-erros-com-tipo-para-os-limites-de-memória-e-de-tabela)
   abaixo). O
   `build()` falha se a memória inicial do módulo já passar do limite. Ao
   contrário do fuel, a memória é de cada instância (em A → B, A e B têm
   memórias separadas), então um limite por pallet é coerente. Três
   detalhes fecham as brechas em volta dele:
   - **Crescer além do limite dá trap** (`trap_on_grow_failure`) em vez de
     devolver -1. Senão, o `alloc` exportado pelo guest (a porta pela qual
     o host pede ao alocador do guest espaço para escrever o input de um
     `Call` ou o resultado de uma host function) devolve o endereço 0, e o
     host escreveria por cima da pilha do guest. O `alloc` do guest SDK
     também dá trap em ponteiro nulo, como segunda proteção.
   - **Uma memória por módulo:** o wasmtime aplica o limite a cada
     memória separadamente, e um módulo pode declarar várias
     (multi-memory, ligado por padrão no wasmtime), multiplicando o
     limite. Guests Rust usam exatamente uma, então o multi-memory é
     desligado na `Engine`, e um módulo com mais de uma memória falha no
     `build()`.
   - **Limite de tabela:** o WASM guarda as funções chamadas por ponteiro
     (`dyn Trait`, ponteiros de `fn`) numa lista numerada, a "table", que
     o compilador cria sozinho. Um módulo pode aumentá-la enquanto roda
     (`table.grow`), e cada item usa memória do nó, fora dos 16 MiB. O
     `Store` limita a **100 mil itens** por tabela; os guests medidos aqui
     nem têm tabela.
4. Mandar um `Call` direto para um `Addr<Pallet>` continua funcionando (o
   `Call` sempre traz um limite de fuel). O rustdoc do `Pallet` avisa que
   isso pula o `CallExecution`: sem span `call.execution` e, depois, sem
   orçamento de fuel compartilhado nem rollback.

Descartadas:

- **Epoch interruption:** não é determinístico.
- **`Engine` fornecida pelo usuário:** não dá para validar, e cada
  operador de nó teria que acertar cinco configurações na mão.
- **Fuel opcional:** um nó esquecer de ligar é exatamente a divergência
  que queremos impedir.
- **Limite de fuel por pallet:** em A → B, não fica claro qual limite vale.
  O orçamento pertence ao `Call` de nível mais alto.
- **Uma segunda `Engine` sem fuel para pallets confiáveis:** agora não. O
  custo medido não justifica (ver Consequências).

## Consequências

- **Custo:** nenhum mensurável nos `Call`s de hoje, em que instanciar
  domina. Por volta de +10–25% na execução WASM pura em código real, e até
  +50–65% no pior caso (laços minúsculos). Só incide na execução do WASM,
  não no salto de thread, na instanciação nem nos ~100 µs ainda sem
  explicação (ADR 0001), que pesam muito mais num `Call`.
- **Uma host function custa 1 de fuel por chamada, faça o que fizer.** O
  fuel só conta instruções WASM: o trabalho do host (um sha256 de 1 MB,
  uma leitura na storage) custa 0, e um `#[host_fn]` só soma o `alloc`
  fixo do guest para devolver o resultado. Host functions rodam nativas,
  então também são bem mais rápidas que o mesmo código em WASM.
- **Oferecer host functions é decisão dos mantenedores da chain.** Quem
  monta a chain escolhe, função por função, entre:
  - **disponibilizar como host function:** rápida, e 1 de fuel por
    chamada, então o fuel não a limita. Os mantenedores da chain passam a
    ser responsáveis pelo custo dela (limitar o tamanho do input, ou
    cobrar: o `Caller` consegue ler e mudar o fuel, ver o
    [TODO](../TODO.pt-BR.md));
  - **deixar a cargo dos mantenedores dos pallets** implementar em WASM:
    mais lenta, mas cada instrução é medida pelo fuel.

  O SDK não impõe nenhuma das duas: é uma troca entre performance e
  medição que pertence a cada chain. O `pallet_example` usa a primeira
  opção com o `crypto::sha256`.
- **Parâmetros de consenso:** o fuel padrão, o limite de memória de cada
  pallet, o limite de tabela e a configuração da `Engine` precisam ser
  iguais em todos os nós.
  Como quem define é quem escreve a chain (todos os nós rodam o mesmo
  binário), basta documentar. A contagem de fuel depende do `.wasm` e do
  custo por instrução do wasmtime: tratar uma atualização do wasmtime como
  possível mudança de consenso até conferir.
- **Fuel ↔ tempo depende da máquina.** A contagem é determinística; quanto
  tempo 100 milhões levam, não. O padrão é uma rede de segurança contra
  laço infinito, não um preço.
- **A profundidade da pilha continua não determinística:** recursão
  profunda já dá trap (não trava), mas o limite de pilha do wasmtime é em
  bytes nativos, então um nó pode dar trap onde outro não dá. Registrado
  no TODO.
- **A API pública muda:** o `PalletBuilder` perde `with_engine`/
  `with_module` e ganha `with_wasm` e `with_memory_bytes`; o `build()`
  devolve `Result`; o `Call` ganha `fuel_limit` e `with_fuel_limit`; o
  `CallResponse` ganha `fuel_used`; o `RuntimeError` ganha `OutOfFuel`. Os exemplos do
  README mudam junto.
- **Ainda sem rollback:** um `Call` que fica sem fuel ou sem memória depois
  de gravar na storage mantém essas gravações, como qualquer outra falha.
  Isso é a fase 5 do [ROADMAP](../ROADMAP.pt-BR.md).
- Se um pallet real mostrar que o custo do fuel importa, a saída é uma
  `Engine` extra sem fuel para pallets confiáveis, com ADR própria.

## Atualização (2026-09-23): erros com tipo para os limites de memória e de tabela

Ficar sem fuel tinha um erro próprio, `RuntimeError::OutOfFuel`, mas
ficar sem memória ou sem itens de tabela era só um `Err` genérico. O
`StoreLimits` do wasmtime reporta esses casos só como texto ("forcing trap
when growing memory to N bytes"), ao contrário do fuel, que chega como o
trap com tipo `Trap::OutOfFuel`.

**Decisão:** trocar o `StoreLimits` pelo `ResourceLimiter` do próprio
SDK, uma struct que implementa o trait de mesmo nome do wasmtime. Quando um crescimento passaria de um limite, ele
devolve um erro com tipo, que o wasmtime transforma em trap e devolve
intacto:

- `RuntimeError::OutOfMemory { memory_bytes }` para a memória do guest;
- `RuntimeError::TableLimitExceeded { max_elements }` para uma tabela.

Um crescimento que o limitador permite, mas que falha mesmo assim (além
do máximo declarado pelo próprio módulo), também dá trap. A regra "trap,
nunca -1" do item 3 continua valendo, agora sem `trap_on_grow_failure`.

**Descartadas:**

- **Identificar o erro pelo texto do wasmtime:** uma atualização do
  wasmtime que mude a frase quebraria a detecção sem aviso.
- **Usar `OutOfMemory` também para tabelas:** a memória do guest não
  acabou; o log apontaria o limite errado.

Nada mais muda: limites, padrões e determinismo continuam os mesmos.

