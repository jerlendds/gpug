<p dir="auto">
  <a href="https://github.com/jerlendds/gpug">
    <img src="./logo.png" height="220px" alt="GPUG logo" align="left" />
  </a>

  <h3 align="left">
    GPUG: GPU Graphs with GPUI
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


As is, GPUG is a proof-of-concept network graph prototype that leverages the GPU for enhanced performance using Zed's gpui library. This approach allows for the visualization and manipulation of complex graphs, making it easier to understand and analyze data connections in real-time. We're currently in the process of turning GPUG into a Rust library.

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

- Ensure Rust is installed - [Rustup](https://rustup.rs/)
- Run gpug with `cargo run`
- Or run gpug with `cargo watch -q -c -w crates/gpug -x 'run -p gpug'`
- To build gpug `cargo build --release`

## Roadmap

| Feature | Description | Completed |
|----------|--------------|------------|
| **Proof-of-concept** | Is it possible to render interactive network graphs easily and quickly with GPUI? | true |
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