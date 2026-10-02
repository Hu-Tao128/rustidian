Tres decisiones clave que quiero destacar (el resto está en el documento):

- El grafo queda detrás de un feature flag de Cargo (--features graph). El binario ligero jamás compila petgraph ni el código de layout; en una máquina más potente activas la feature sin tocar el resto del código.
- NoteId = ruta del archivo para v1, con una limitación aceptada a propósito: renombrar rompe enlaces (igual que Obsidian sin plugins). Está documentado como trade-off consciente, no como descuido.
- Para cuando llegues al grafo: Slint no tiene un Canvas como Qt Quick, así que la vía más barata es que Rust calcule el layout y lo entregue como imagen (SVG o buffer), no dibujo vectorial en vivo.

# Rustidian — plan de arquitectura y desarrollo

Documento de referencia para construir Rustidian: un editor de notas Markdown ultraligero en Rust + Slint, pensado para correr bien en un ThinkPad T60 (Core 2 Duo, ~2.9 GiB RAM, IceWM/X11, GPU sin aceleración moderna) y hardware similar.

---

## 1. Visión y principios de diseño

1. **Ligereza primero.** Cada decisión técnica se evalúa contra: ¿esto compila rápido en un Core 2 Duo? ¿esto usa RAM/CPU de más sin necesidad?
2. **Núcleo desacoplado de la interfaz.** La lógica (archivos, Markdown, enlaces) no sabe que existe una UI. Si mañana cambias Slint por otra cosa, el núcleo no se toca.
3. **Texto plano, siempre.** Las notas son `.md` normales en una carpeta normal. Cero bases de datos, cero formatos propietarios.
4. **YAGNI (You Aren't Gonna Need It).** No se construye nada "por si acaso". El grafo, la sincronización, los plugins — todo eso se agrega cuando haga falta, no antes.
5. **Lo costoso es opcional y aislado.** Todo lo que consuma recursos de forma no trivial (el grafo de conexiones es el caso principal) vive detrás de un _feature flag_ de Cargo, para que el binario por defecto ni siquiera lo compile.

---

## 2. Arquitectura (resumen)

Workspace de Cargo con dos crates:

- **`rustidian-core`**: lógica pura. No importa `slint` ni ningún crate de UI. Se puede compilar y testear sin abrir una sola ventana.
- **`rustidian-ui`**: el binario final. Depende de `rustidian-core` y de `slint`. Traduce entre los tipos del core y los tipos que Slint entiende, y es el único lugar donde "vive" la ventana.

Regla de oro: **la UI nunca toca el disco directamente**. Todo `fs::read`, `fs::write`, escaneo de carpetas, etc. pasa por `rustidian-core::vault`.

---

## 3. Estructura de carpetas

```
rustidian/
├── Cargo.toml                     # workspace virtual
├── .gitignore
├── README.md
├── CHANGELOG.md
│
├── rustidian-core/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs                 # re-exporta los módulos públicos
│   │   ├── error.rs                # CoreError (thiserror)
│   │   ├── config.rs               # ruta del vault, preferencias del usuario
│   │   ├── vault.rs                # CRUD de notas + escaneo de carpeta
│   │   ├── markdown.rs             # wrapper sobre pulldown-cmark
│   │   ├── links.rs                # wikilinks [[Nota]] + backlinks
│   │   ├── search.rs               # búsqueda por texto
│   │   └── graph.rs                # #[cfg(feature = "graph")] — layout del grafo
│   └── tests/
│       ├── vault_tests.rs
│       ├── markdown_tests.rs
│       └── links_tests.rs
│
├── rustidian-ui/
│   ├── Cargo.toml
│   ├── build.rs                    # compila los .slint
│   ├── src/
│   │   ├── main.rs                 # entry point, conecta core <-> UI
│   │   ├── bridge.rs                # traduce tipos core -> tipos Slint
│   │   └── worker.rs                # hilo secundario para tareas largas
│   └── ui/
│       ├── main.slint
│       ├── sidebar.slint
│       ├── editor.slint
│       ├── preview.slint
│       └── graph_view.slint         # se compila siempre; el botón que la abre se oculta si la feature "graph" está apagada
│
└── vault-ejemplo/                  # notas de prueba para desarrollo — nunca el vault real del usuario
    └── bienvenida.md
```

---

## 4. Reglas de arquitectura (no negociables)

1. `rustidian-core` **nunca** importa `slint` ni ningún crate de UI. Verifícalo de vez en cuando con `cargo tree -p rustidian-core`.
2. Toda operación de disco vive en `core::vault`. La UI llama funciones del core, nunca `std::fs` directamente.
3. La UI **nunca bloquea el hilo principal**. Cualquier operación que pueda tardar (escanear un vault grande, indexar enlaces) corre en `std::thread` y regresa a la UI vía `slint::invoke_from_event_loop`.
4. Cero `.unwrap()` / `.expect()` fuera de `main.rs` y de tests. En el resto, todo error se propaga con `Result<T, CoreError>` y `?`.
5. Un solo tipo de error por crate. En `core`, `CoreError` con `thiserror`. La UI convierte esos errores a mensajes legibles, nunca los ignora en silencio.
6. Toda funcionalidad opcional (el grafo) vive detrás de `#[cfg(feature = "...")]` en su propio módulo — nunca mezclada línea por línea con el código base.
7. Ninguna dependencia nueva sin justificar por qué `std` no alcanza. Cada crate que agregas es tiempo de compilación en una máquina de dos núcleos.
8. Toda función pública de `core` lleva un doc-comment (`///`) y, si tiene lógica no trivial (parseo, regex, condicionales), un test.
9. `cargo fmt --check` y `cargo clippy --all-targets --all-features -- -D warnings` deben pasar limpios antes de cualquier commit.
10. Identificadores en inglés (`fn read_note`, no `fn leer_nota`); los comentarios pueden ser en español. No mezcles los dos idiomas dentro de un mismo nombre.

---

## 5. Convenciones de código

- **Formato**: `rustfmt` con la configuración por defecto. No discutir estilos línea por línea — lo decide la herramienta.
- **Nombres**: `snake_case` para funciones y variables, `PascalCase` para tipos, `SCREAMING_SNAKE_CASE` para constantes.
- **Módulos**: un archivo por responsabilidad (`vault.rs`, `markdown.rs`, `links.rs`), no un único `lib.rs` gigante. Evita `mod.rs`; usa `nombre_modulo.rs` directo (estilo de edición 2018+).
- **Funciones**: si una función pasa de ~40-50 líneas o hace más de una cosa, es candidata a dividirse.
- **Comentarios**: explican el _por qué_, no el _qué_ (el código ya dice qué hace). Ejemplo útil: `// Normalizamos a minúsculas porque los wikilinks no distinguen mayúsculas`.
- **Nada de dependencias "por si acaso"**: antes de un `cargo add`, pregúntate si `std` ya resuelve el problema (por ejemplo, `std::sync::OnceLock` en vez de `once_cell` — ya no hace falta desde Rust 1.70).

---

## 6. Manejo de errores

Un enum de error por crate, con `thiserror` (usa la versión 2, que es la actual):

```rust
// rustidian-core/src/error.rs
#[derive(thiserror::Error, Debug)]
pub enum CoreError {
    #[error("no se pudo leer o escribir: {0}")]
    Io(#[from] std::io::Error),

    #[error("la nota '{0}' no existe")]
    NotFound(String),

    #[error("ya existe una nota con ese nombre: {0}")]
    NameCollision(String),

    #[error("no se pudo interpretar la configuración: {0}")]
    Config(String),
}
```

Regla práctica: si una función de `core` puede fallar, devuelve `Result<T, CoreError>`. La UI hace `match` sobre el resultado y muestra un mensaje al usuario — nunca deja que un error se pierda ni hace panic por una operación de archivo fallida (por ejemplo, un permiso denegado no debería tumbar la app).

---

## 7. Modelo de datos central

```rust
// rustidian-core/src/vault.rs
pub type NoteId = String; // ruta relativa dentro del vault, ver limitación abajo

pub struct NoteMeta {
    pub id: NoteId,
    pub title: String, // nombre de archivo sin extensión
}

pub struct Note {
    pub meta: NoteMeta,
    pub content: String,
}
```

```rust
// rustidian-core/src/links.rs
use std::collections::HashMap;

pub struct LinkIndex {
    pub outgoing: HashMap<NoteId, Vec<NoteId>>,
    pub backlinks: HashMap<NoteId, Vec<NoteId>>,
}
```

**Limitación conocida y aceptada para v1**: `NoteId` es la ruta/nombre del archivo. Si renombras una nota, los `[[enlaces]]` que apuntaban a ella quedan rotos — es el mismo comportamiento que tiene Obsidian sin plugins. No lo resuelvas ahora (agregar IDs estables vía metadata sería sobre-ingeniería para v1); anótalo como mejora futura si algún día te molesta de verdad.

---

## 8. Feature opcional: el grafo

Objetivo: que el binario por defecto (el que usas en el T60) **no cargue `petgraph` ni ningún código de layout**, pero que activarlo en una máquina con más recursos sea un flag de compilación, no una reescritura.

**`rustidian-core/Cargo.toml`**

```toml
[dependencies]
pulldown-cmark = "0.13"
thiserror = "2"
dirs = "5"
walkdir = "2"
regex = "1"
serde = { version = "1", features = ["derive"] }
toml = "0.8"

[dependencies.petgraph]
version = "0.6"
optional = true

[dev-dependencies]
tempfile = "3"

[features]
graph = ["dep:petgraph"]
```

**`rustidian-ui/Cargo.toml`**

```toml
[dependencies]
rustidian-core = { path = "../rustidian-core" }
slint = { version = "1.18", default-features = false, features = ["backend-winit", "renderer-software", "compat-1-18"] }
dirs = "5"
# Selector de carpeta nativo (backend GTK3, disponible en X11).
rfd = { version = "0.17", default-features = false, features = ["gtk3"] }

[build-dependencies]
slint-build = "1.18"

[features]
graph = ["rustidian-core/graph"]
```

**Dependencias del sistema (Linux).** Además del toolchain de Rust (instalado
con el script de `rustup`, no con el gestor de paquetes de la distro), la
compilación necesita: un toolchain de C (`gcc`/`make`), `pkg-config`, las
cabeceras de desarrollo de **GTK3** (para `rfd`), **xkbcommon**, **xcb**
(`shape` + `xfixes`) y **fontconfig** (para el backend `winit` de Slint). El
comando de instalación para Arch, Debian/Ubuntu y Fedora/RHEL está en el
[README, sección *Requirements*](README.md#requirements). Se requiere **Rust
1.92 o superior** (Slint 1.18 usa la edición 2024).

Cómo se organiza en la práctica:

- `rustidian-core/src/graph.rs` completo detrás de `#[cfg(feature = "graph")]`: calcula un layout tipo Fruchterman-Reingold **una sola vez** cuando el usuario abre esa vista, no en cada frame.
- `graph_view.slint` se compila siempre (es barato, solo define la estructura visual), pero el botón que la abre en el sidebar se muestra u oculta con una propiedad booleana que Rust fija en tiempo de compilación: `ui.set_graph_available(cfg!(feature = "graph"));`. Así el binario ligero nunca descarga ni compila `petgraph`, aunque el archivo `.slint` exista.
- **Nota técnica importante**: Slint no tiene un `Canvas` de dibujo libre como Qt Quick. Para pintar nodos y líneas, la forma más simple y barata es que Rust calcule las posiciones y genere una imagen (SVG o un buffer con `tiny-skia`/`plotters`) que se muestra con el elemento `Image` de Slint, regenerándola solo cuando cambie el layout — no en cada repintado.
- Build normal (ligero): `cargo build --release -p rustidian-ui`
- Build con grafo: `cargo build --release -p rustidian-ui --features graph`

---

## 9. Presupuesto de recursos y reglas de rendimiento

- **Objetivo de RAM**: <50 MB en reposo, <100 MB con un vault de 500 notas cargado (solo metadata en memoria, no todo el contenido).
- **Carga perezosa**: al iniciar, solo se lee nombre + ruta de cada nota (`vault::list_notes`). El contenido de una nota se lee al abrirla, no antes.
- **Autosave con debounce**: usa `slint::Timer` en modo _single-shot_, reiniciado en cada tecla, con 500-800 ms de espera antes de escribir a disco. El T60 probablemente tiene disco mecánico — escribir en cada tecla se siente.
- **Hilos**: cualquier operación que pueda tardar más de ~50 ms (escaneo inicial de un vault grande, reconstrucción del índice de enlaces, búsqueda en contenido) corre en `std::thread::spawn`, y el resultado vuelve a la UI con `slint::invoke_from_event_loop`. Slint no es thread-safe: nunca toques una propiedad de la UI desde otro hilo directamente.
- **Sin animaciones por defecto**: el renderer de software de Slint repinta más lento que uno acelerado por GPU; evita transiciones y easing salvo que agreguen valor real.
- **Mide siempre en `--release`**: un build de debug en un Core 2 Duo puede ser 5-10x más lento y te va a dar conclusiones equivocadas sobre rendimiento.

---

## 10. Testing y control de calidad

- Cada módulo de `core` con lógica no trivial tiene tests unitarios: `vault.rs` (crear/leer/renombrar/borrar, nombres duplicados), `markdown.rs` (casos básicos de Markdown), `links.rs` (enlaces simples, con alias, rotos, y el caso A↔B para confirmar que no hay recursión infinita).
- No se necesitan tests de UI por ahora — probar la UI es manual, en el T60 real.
- Antes de cada commit: `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --workspace`.
- Usa `vault-ejemplo/` (notas ficticias) para desarrollo, nunca tu vault real, para no arriesgar tus notas mientras pruebas código nuevo.

---

## 11. Roadmap detallado

### V0 — Esqueleto que compila y corre

1. `cargo new --lib rustidian-core` y `cargo new rustidian-ui`; `Cargo.toml` raíz como workspace (`[workspace] members = ["rustidian-core", "rustidian-ui"]`).
2. Agregar dependencias base (ver tabla de la sección 8, sin la parte de `graph` todavía).
3. Definir `CoreError` mínimo: `Io`, `NotFound`.
4. Implementar en `vault.rs`: `list_notes(path) -> Result<Vec<NoteMeta>, CoreError>` (solo metadata), `read_note`, `write_note`, `create_note`, `delete_note`.
5. Implementar `markdown::to_html(&str) -> String` (wrapper directo sobre `pulldown_cmark`).
6. UI: ventana con sidebar (lista de notas), editor (`TextInput`), preview (`Text` con `wrap: word`).
7. Conectar callbacks: click en nota → leer del core → mostrar; guardar → escribir con el core → refrescar preview.
8. Resolver la ruta del vault con `dirs::home_dir()`, nunca con una ruta relativa tipo `./mi_boveda`.

**Criterio de aceptación**: puedes crear, ver, editar y guardar una nota `.md` desde la UI, corriendo en el T60 en modo `--release`.

**Errores comunes a evitar**: rutas relativas para el vault; sacar conclusiones de rendimiento de un build en modo debug; bloquear la UI leyendo archivos sin plan de moverlo a un hilo después.

---

### V1 — CRUD robusto + autosave

1. Renombrar nota: mover el archivo, verificar que el nuevo nombre no colisione con uno existente (`CoreError::NameCollision`).
2. Confirmación antes de borrar (evitar pérdida accidental).
3. Autosave con debounce de 500-800 ms vía `slint::Timer`.
4. Errores visibles en la UI: si falla una escritura, mostrar un mensaje — nunca fallar en silencio ni hacer panic.
5. Persistir la ruta del vault en `~/.config/rustidian/config.toml` (parseo simple, sin necesidad de `serde` si quieres cero dependencias extra — un `HashMap<String,String>` a mano alcanza para una sola clave).
6. Tests unitarios de `vault.rs` cubriendo los cinco puntos anteriores.

**Criterio de aceptación**: cerrar y reabrir la app conserva el vault configurado; nada se pierde al escribir.

**Errores comunes**: renombrar sin chequear colisión; guardar en cada `onTextChanged` sin debounce (notorio en disco mecánico).

---

### V2 — Wikilinks y backlinks

1. Regex para `[[Nota]]` y `[[Nota|alias]]`, compilada **una sola vez** con `std::sync::OnceLock` (nunca dentro de un loop).
2. `links::build_index(vault_path) -> LinkIndex`: recorre el vault una vez, arma `outgoing`, deriva `backlinks` invirtiendo el mapa.
3. Reconstruir el índice solo al guardar una nota, no en cada tecla.
4. Decide y documenta una normalización de nombres (por ejemplo `to_lowercase().trim()`) para comparar enlaces de forma consistente.
5. UI: panel de "Backlinks" mostrando qué notas enlazan a la actual.
6. Enlaces rotos (apuntan a una nota inexistente): no deben crashear; basta con no mostrarlos como válidos por ahora.
7. Tests: enlaces simples, con alias, rotos, y el caso A↔B (no debe entrar en recursión — es una tabla, no un recorrido).

**Criterio de aceptación**: escribes `[[OtraNota]]`, guardas, y en "OtraNota" aparece la primera como backlink.

**Errores comunes**: reconstruir todo el índice en cada tecla; comparar nombres sin normalizar (`Nota` vs `nota` tratadas como distintas por accidente).

---

### V3 — Búsqueda

1. `search::find(query, vault_path) -> Vec<SearchResult>` con `walkdir` + comparación de substring — no hace falta indexar para un vault personal de cientos de notas.
2. Decide si la búsqueda es solo por título o también por contenido; si es por contenido, léelo archivo por archivo (no cargues todo el vault en RAM de una vez), y considera moverla a un hilo si el vault crece.
3. UI: input de búsqueda + lista de resultados clicable.
4. Debounce de 150-200 ms en el input para no re-buscar en cada letra.

**Criterio de aceptación**: buscar una palabra presente en 3 de 50 notas responde en el T60 sin demora perceptible.

**Errores comunes**: sin debounce, cada tecla dispara un recorrido completo del vault.

---

### V4 — Pulido de UX

1. Atajos de teclado: guardar (Ctrl+S), nueva nota (Ctrl+N).
2. Indicador de "guardado / cambios sin guardar".
3. Manejo del primer arranque (vault vacío, pedir o crear una carpeta).
4. Probar con un vault grande (miles de notas) — el objetivo no es que sea rápido, sino que no crashee ni se congele.
5. Empaquetado: script de build simple + un archivo `.desktop` para que aparezca en el menú de IceWM.

---

### V5 — Grafo (opcional, solo si la máquina lo aguanta)

1. Compilar con `--features graph` en ambos crates.
2. Implementar `graph::layout(&LinkIndex) -> Vec<NodePosition>` con Fruchterman-Reingold (no hace falta nada más sofisticado).
3. Calcular el layout una sola vez al abrir la vista; recalcular solo si el usuario lo pide explícitamente.
4. Dibujar generando una imagen (SVG o buffer con `tiny-skia`/`plotters`) mostrada en un `Image` de Slint, en vez de intentar un dibujo vectorial en vivo — Slint no tiene equivalente directo al `Canvas` de Qt Quick.
5. Prueba específica en el T60: si con 200+ notas el layout tarda más de 1-2 segundos o la RAM sube demasiado, está bien dejar esta vista como "función de escritorio" y no forzarla en la máquina vieja — la meta del proyecto es que lo esencial corra bien en hardware antiguo, no que todo corra en todos lados.

---

## 12. Checklist antes de cada commit

- [ ]  `cargo fmt --check`
- [ ]  `cargo clippy --all-targets --all-features -- -D warnings`
- [ ]  `cargo test --workspace`
- [ ]  Si tocaste UI o algo sensible a rendimiento: probarlo en el T60 real, en `--release`
- [ ]  Si agregaste una feature visible: una línea en `CHANGELOG.md`

---

## 13. Actualización V6 — subcarpetas, preview por bloques, edición y temas

Esta sección documenta la segunda iteración del proyecto. No invalida los
principios de las secciones 1–12: siguen vigentes.

### 13.1 Subcarpetas en el vault

- `NoteId` **no cambia**: sigue siendo la ruta relativa al vault (ahora puede
  incluir subcarpetas, p. ej. `"Proyectos/2024/nota.md"`).
- `vault::list_notes` escanea recursivamente con `walkdir::WalkDir` y filtra
  solo archivos `.md`.
- Se agrega `vault::FolderNode` y `vault::list_notes_tree`, que agrupan las
  notas por carpeta padre en un árbol simple (nombre + notas propias +
  subcarpetas hijas). Devuelve un único nodo raíz (`path == ""`).
- `vault::create_note_in(vault, folder, title)` crea la nota en una carpeta
  (creando los directorios intermedios) y `rename_note` conserva la carpeta.
- En la UI, el sidebar es un árbol expandible/colapsable. El estado de
  expansión vive en `AppData::expanded` (no está hardcodeado) y la carpeta
  seleccionada en `AppData::selected_folder`; las notas nuevas se crean ahí.
- **Pendiente (mejora futura):** arrastrar y soltar para mover notas.

### 13.2 Preview de Markdown por bloques

- Ya no se usa `pulldown_cmark::html::push_html()`. `markdown::parse_blocks`
  consume los eventos del `Parser` y construye un árbol propio `Block`/`Inline`
  (ver `rustidian-core/src/markdown.rs`).
- El core expone el árbol; la UI lo aplana a un modelo `[BlockItem]` y lo
  renderiza con un componente por variante: `heading.slint`,
  `paragraph.slint`, `list_item.slint`, `task_item.slint` (CheckBox real),
  `code_block.slint` (fondo + monoespaciada), `block_quote.slint`,
  `table_block.slint` (GridLayout real) y `thematic_break.slint`.
- El contenido inline se serializa a CommonMark y se renderiza con
  `StyledText` (negritas, cursivas, tachado, código inline y enlaces).
- Nota de prueba exhaustiva: `vault-ejemplo/00 Markdown prueba.md`.

### 13.3 Asistencia de edición

Vive **solo en `rustidian-ui`** (`src/editor_assist.rs` + `ui/editor.slint`);
no toca `rustidian-core`. El editor usa el `TextInput` de bajo nivel para
acceder a los offsets de cursor/selección.

1. **Auto-continuar listas** con Enter (`- item`, `1. item`, `- [ ] task`);
   un ítem vacío sale de la lista.
2. **Auto-cerrar pares** `**`, `_`, `` ` `` y `[[`, envolviendo la selección si
   hay texto seleccionado.
3. **Autocompletado de wikilinks**: al escribir `[[` se muestra una lista
   filtrada de títulos existentes; Tab o Enter insertan `[[Nombre]]`.

### 13.4 Temas Catppuccin

- `ui/theme.slint` define `global Palette` con exactamente un rol por color
  semántico: `bg`, `surface`, `card`, `border`, `text`, `text-muted`, `accent`,
  `success`, `danger`, `warning`. **Regla estricta:** ningún otro `.slint` usa
  hex directo.
- Valores Catppuccin:

  | Rol | Mocha | Latte |
  |---|---|---|
  | bg | `#1e1e2e` base | `#eff1f5` base |
  | surface | `#181825` mantle | `#e6e9ef` mantle |
  | card | `#313244` surface0 | `#ccd0da` surface0 |
  | border | `#45475a` surface1 | `#bcc0cc` surface1 |
  | text | `#cdd6f4` text | `#4c4f69` text |
  | text-muted | `#a6adc8` subtext0 | `#6c6f85` subtext0 |
  | accent | `#74c7ec` sapphire | `#209fb5` sapphire |
  | success | `#a6e3a1` green | `#40a02b` green |
  | danger | `#f38ba8` red | `#d20f39` red |
  | warning | `#f9e2af` yellow | `#df8e1d` yellow |

- Mocha (oscuro) y Latte (claro); `apply_theme()` en Rust sobreescribe el
  `Palette`.
- Botón en la barra y atajo `Ctrl+T`; la elección se persiste como
  `dark_mode` en `config.toml`.

### 13.5 Selector de carpeta nativo

El primer arranque y el botón "Change vault…" abren el explorador de archivos
nativo con `rfd` (backend GTK3), en vez de pedir una URL escrita.

### 13.6 Reglas que se mantienen

- `rustidian-core` sigue sin importar `slint`.
- La UI no llama `std::fs` directamente.
- Toda dependencia nueva (`rfd`) está justificada: no hay API de diálogos
  nativos en `std` y Slint no la incluye.
- `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`
  y `cargo test --workspace` deben pasar limpios.