![status](https://img.shields.io/badge/status-experimental-orange)

<p dir="auto">
  <a href="https://github.com/jerlendds/gpug">
    <img src="./logo.svg" height="220px" alt="GPUG logo" align="left" />
  </a>

  <h3 align="left">
    GPUG: A GPU-accelerated graph visualization engine built with <a href="https://zed.dev/">Zed's GPUI</a>, exploring how far Rust and GPUI can take interactive graph rendering.
  </h3>


  <p align="left">

  *The origins of graph theory are humble, even frivolous.*

  ~ [*Norman L. Biggs*](https://en.wikipedia.org/wiki/Norman_L._Biggs)
  </p>
  <br/>
</p>


---

# gpug

GPUG is an initial prototype exploration in building a high-performance, interactive network graph visualization library that leverages the GPU via Zed's gpui library. This approach might allow for the visualization and manipulation of complex graphs, and serves as a foundation for a future Rust library focused on graph interactivity and visualization.

[gpui-network-graph.mp4](https://github.com/user-attachments/assets/75b3a6d1-3cf1-42c2-9dc7-1f48b570e9bd)

Created with Create GPUI App, to get started with GPUI visit the GPUI documentation or examples provided:

- [`gpui`](https://www.gpui.rs/)
- [gpui documentation](https://github.com/zed-industries/zed/tree/main/crates/gpui/docs)
- [gpui examples](https://github.com/zed-industries/zed/tree/main/crates/gpui/examples)
- [gpui book](https://matinaniss.github.io/gpui-book/introduction.html)
- [gpui HN threads](https://duckduckgo.com/?t=ffab&q=%22gpui%22%20site%3Anews.ycombinator.com&ia=web)
- [gpui tutorial](https://github.com/hedge-ops/gpui-tutorial)
- [gpui component library](https://github.com/longbridge/gpui-component)
- [gpui music player](https://github.com/143mailliw/hummingbird)
- [gpui examples visualized](https://studium.dev/tech/playing-gpui-rust)


- [gpug notes from jerlendds](https://studium.dev/tech/gpui-networks)


## Usage

0. Ensure Rust is installed ~ [Rustup](https://rustup.rs/)

1. To hack on gpug
   ```bash
   git clone https://github.com/jerlendds/gpug
   cd gpug
   cargo run --example kitchen_sink
   # or to watch:
   # cargo install cargo-watch
   cargo watch -x "run --example kitchen_sink"
   ```

2. To build gpug
   ```bash
   cargo build --release
   ```


## Roadmap

| Feature | Description | Completed |
|----------|--------------|------------|
| **Proof-of-concept** | Is it possible to render interactive network graphs with GPUI? | true |
| **[`petgraph`](https://lib.rs/crates/petgraph) or [`graph`](https://lib.rs/crates/graph) backends + more?** | Choose a graph backend fit for your usecase | false |
| **Large Graph Handling** | Capable of visualizing graphs with thousands of nodes/edges? | false |
| to be continued | at a later date... | false |
