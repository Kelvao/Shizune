# Planejamento de Desenvolvimento: Deep Virtual Mic (Rust)

## 1. Requisitos do Sistema

**Funcionais**
* **Captura:** Identificar e ler o fluxo de áudio do microfone físico selecionado via `cpal`.
* **Processamento:** Executar a inferência do modelo DeepFilterNet puro (via engine `tract`) em tempo real.
* **Controle de Atenuação (Força do Filtro):** Utilizar o parâmetro nativo de limite de atenuação da rede neural para ajustar a intensidade da redução de ruído, preservando a integridade e a fase original da voz.
* **Saída Dinâmica:** Injetar o fluxo no servidor de áudio do SO, expondo-o como um microfone virtual.
* **Compatibilidade e Fallback:** Priorizar Node virtual no PipeWire; caso o socket não seja detectado, realizar fallback para Null Sink no PulseAudio.
* **Interface Gráfica (GUI):** O projeto utilizará a biblioteca Slint para prover uma interface visual para controles e ajustes.

**Não-Funcionais**
* **Latência:** Atraso total (Hardware -> IA -> Virtual Node) estritamente inferior a 40ms.
* **Uso de CPU:** Execução em thread única para o motor de IA, sem picos de context switching.
* **Concorrência Lock-Free:** Thread de áudio livre de travamentos. Atualizações de parâmetros devem usar atômicos ou canais bounded.
* **GUI Renderization:** A interface gráfica (Slint) deve rodar em thread separada com aceleração por GPU, não interferindo nas threads de I/O de áudio.

## 2. Arquitetura e Fluxo de Dados (Topologia Tripla)

O design desacopla I/O da regra de negócio usando dois Ring Buffers para transição lock-free.

1. **Camada de Entrada (cpal):** Callback de captura empurra as amostras brutas (f32) para o *Ring Buffer de Entrada*.
2. **Core / Motor de IA (Thread Dedicada):** 
   * Consome o *Ring Buffer de Entrada* quando atinge o tamanho do frame (ex: 480 samples).
   * Executa a inferência respeitando o limite de atenuação nativo configurado (sem mistura Dry/Wet).
   * Empurra o resultado processado para o *Ring Buffer de Saída*.
3. **Camada de Saída:**
   * **Rota PipeWire:** Instancia um nó `Audio/Source/Virtual` consumindo o *Ring Buffer de Saída*.
   * **Rota PulseAudio:** Consome o buffer via `cpal` (playback) apontando para um Null Sink criado dinamicamente via `pactl`.

## 3. Fases de Desenvolvimento

**Fase 1: Fundação e Dependências**
* Setup do `Cargo.toml` com `cpal`, `pipewire`, `deep-filter`, `ringbuf` e `slint`.
* Criação das estruturas de gerenciamento de estado sem alocação dinâmica.

**Fase 2: Motor de IA e Gerenciamento de Buffers**
* Pré-alocação dos Ring Buffers no heap (`HeapRb::<f32>::new(4800)`).
* Implementação do produtor lock-free no callback de captura.
* Loop de processamento com micro-yield (`thread::yield_now()`) para consumo exato do frame.
* Configuração da máscara de limite de atenuação nativa na API do DeepFilterNet.

**Fase 3: Handshake de Servidor (Fallback)**
* Detecção passiva do socket em `/run/user/$(id -u)/pipewire-0`.
* Implementação da lógica condicional de saída (PipeWire Node vs. pactl Null Sink).
* Mapeamento de encerramento gracefully para descarregar módulos do PulseAudio se necessário.

**Fase 4: Tratamento de Falhas e Otimização**
* Proteção contra *Buffer Underrun* (injetar silêncio se a IA atrasar).
* Reconexão automática em caso de desconexão física do hardware (sem panic).
* Prevenção rigorosa de alocações dinâmicas (`Vec::new`, `String::from`) dentro dos callbacks de áudio.

**Fase 5: Interface Gráfica (Slint)**
* Implementação do painel de controle utilizando a biblioteca Slint.
* Vinculação (binding) bidirecional segura entre a GUI e as variáveis atômicas de estado (limite de atenuação) compartilhadas com a Engine.

## 4. Arquitetura (Hexagonal / Ports and Adapters)

O sistema utiliza a Arquitetura Hexagonal (Ports and Adapters) para isolar completamente a complexidade de hardware (I/O) da regra de negócio (motor de IA). Essa abordagem é essencial porque APIs de áudio no Linux possuem ciclos de vida agressivos e callbacks que podem dificultar os testes se acoplados à lógica principal. O sistema é dividido em três camadas principais:

1. **Domain (Regra de Negócio):** 
   O núcleo do sistema. Não possui conhecimento sobre hardware, APIs de sistema operacional ou servidores de áudio. Consome as amostras do Ring Buffer de entrada, executa a inferência (`tract`) aplicando a atenuação nativa diretamente na máscara de frequência, e escreve no Ring Buffer de saída. É altamente testável e focado unicamente na manipulação dos arrays de áudio.

2. **Ports (Contratos):**
   Definição de interfaces (Traits no Rust) que estipulam os contratos de como o áudio entra (Source) e sai (Sink) do Domain. Isso garante que a regra de negócio exija apenas dados puros, permitindo até testes unitários com arrays de áudio simulados.

3. **Adapters (Infraestrutura):**
   As implementações concretas das portas que lidam com o mundo exterior:
   * **Adaptador de Entrada:** Implementa a porta conectando e empurrando o áudio do microfone real (via `cpal`) para o Ring Buffer primário.
   * **Adaptadores de Saída:** Consomem o Ring Buffer secundário e roteiam para o sistema operacional, tendo implementações separadas para criar o nó virtual via PipeWire ou o fallback via PulseAudio.
   * **Adaptador de UI (Slint):** Implementa os controladores da interface para enviar alterações ao Domain, mantendo a thread visual separada e delegando carga para a GPU.

## 5. Padrões de Desenvolvimento

É estritamente obrigatório aplicar as práticas de **Clean Code** e os princípios **SOLID** em toda a base de código:
* **Single Responsibility (SRP):** Cada adaptador (cpal, pipewire, pulse, slint) deve lidar exclusivamente com sua respectiva API de I/O.
* **Open/Closed (OCP):** A arquitetura deve permitir a adição de novos adaptadores de saída sem alterar o núcleo da aplicação.
* **Dependency Inversion (DIP):** O domínio (`engine.rs`) deve depender apenas das interfaces (Traits), nunca das implementações diretas de hardware.
* Nomenclatura expressiva, funções curtas e ausência de *magic numbers* (ex: taxas de amostragem fixas no código sem constantes claras).
