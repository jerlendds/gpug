![status](https://img.shields.io/badge/status-experimental-orange)

<p dir="auto">
  <a href="https://github.com/jerlendds/gpug">
    <img src="./logo.png" height="220px" alt="GPUG logo" align="left" />
  </a>

  <h3 align="left">
    GPUG: A GPU-accelerated graph visualization engine built with Zed’s GPUI, exploring how far Rust and Zed's GPUI can take interactive graph rendering.
  </h3>


  <p align="left">

  *The origins of graph theory are humble, even frivolous.*

  ~ [*Norman L. Biggs*](https://en.wikipedia.org/wiki/Norman_L._Biggs)
  </p>
  <br/>
</p>

---

---

# gpug

GPUG is a prototype for a high-performance, interactive network graph visualization library that leverages the GPU via Zed's gpui library. This approach allows for the visualization and manipulation of complex graphs, and serves as a foundation for a future Rust library focused on graph interactivity and visualization.

[gpui-network-graph.mp4](https://github.com/user-attachments/assets/75b3a6d1-3cf1-42c2-9dc7-1f48b570e9bd)

Created with Create GPUI App, to get started with GPUI visit the GPUI documentation or examples provided:

- [`gpui`](https://www.gpui.rs/)
- [GPUI documentation](https://github.com/zed-industries/zed/tree/main/crates/gpui/docs)
- [GPUI examples](https://github.com/zed-industries/zed/tree/main/crates/gpui/examples)
- [GPUI book](https://matinaniss.github.io/gpui-book/introduction.html)
- [GPUI HN threads](https://duckduckgo.com/?t=ffab&q=%22gpui%22%20site%3Anews.ycombinator.com&ia=web)
- [GPUI tutorial](https://github.com/hedge-ops/gpui-tutorial)
- [GPUI component library](https://github.com/longbridge/gpui-component)
- [GPUI music player](https://github.com/143mailliw/hummingbird)
- [GPUI notes from jerlendds](https://studium.dev/tech/playing-gpui-rust)

## Usage

0. Ensure Rust is installed ~ [Rustup](https://rustup.rs/)

1. To hack on gpug
   ```bash
   git clone https://github.com/jerlendds/gpug
   cd gpug
   cargo run
   # or to watch:
   cargo watch -q -c -w crates/gpug -x 'run -p gpug
   ```

2. To build gpug
   ```bash
   cargo build --release
   ```


## Roadmap

| Feature | Description | Completed |
|----------|--------------|------------|
| **Proof-of-concept** | Is it possible to render interactive network graphs with GPUI? | true |
| **Built-in Node & Edge Types** | GPUG ships with default node and edge types (e.g. `default`, `smoothstep`, `step`, `straight`) but supports full customization. | false |
| **Custom Nodes & Edges** | You can define fully custom nodes (with arbitrary rendering, embedded elements) and edges with custom behavior and style. | false |
| **Handles / Ports** | Connection handles (ports) can be placed on any side or position, styled arbitrarily, enabling multiple inputs/outputs per node. | false |
| **Interactive Connection / Drag to Connect** | Users can drag from one handle to another to create new edges, with placeholder "connection line" behavior. | false |
| **Viewport Control & Animation** | GPUG supports controlling the viewport (position, zoom) programmatically, and animating or constraining transforms. | false |
| **Minimap, Controls, Background** | Out-of-the-box UI components like a minimap, pan/zoom controls, and grid background are included. | false |
| **Animating Node / Edge Properties** | You can animate transitions of color, size, or position of nodes/edges. | false |
| **Large Graph Handling** | Capable of visualizing graphs with thousands of nodes/edges in the browser. | false |
| **Hands-on Examples** | Learn by example with various applications showcased. | false |
| **Comprehensive Documentation** | Access thorough documentation | false |
| **Helpful Resources** | Refer to the GPUG book for in-depth guidance. | false |
