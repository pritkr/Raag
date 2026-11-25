//! Pure Rust NSH Live Preview Host (`nsh_preview`).
//!
//! Runs the REAL Pure Rust NSH `resonance::App` (`nsh-engine =0.7.0`, `nsh-core`,
//! `nsh-base`, `theme.json`, `src/screens/*.rs`, `src/store.rs`) inside a real
//! `nsh_engine::Engine` instance, captures every `MountBatch` frame emitted by
//! NSH's Taffy layout & reactive signal engine, and serves the live Native View
//! Tree + bidirectional event dispatch over HTTP (`127.0.0.1:3001`).
//!
//! This ensures 100% of the UI, state, theming, layout, and user journeys shown
//! in the AI Studio preview are powered directly by Pure Rust + NSH — with zero
//! duplicated screens or mock state in TypeScript.

#[cfg(target_os = "android")]
fn main() {}

#[cfg(not(target_os = "android"))]
mod host {
    use std::collections::HashMap;
    use std::convert::Infallible;
    use std::net::SocketAddr;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use bytes::Bytes;
    use crossbeam_channel::{Receiver, Sender};
    use http_body_util::{BodyExt, Full};
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Method, Request, Response, StatusCode, header};
    use hyper_util::rt::TokioIo;
    use nsh_core::platform::Os;
    use nsh_core::{
        ApproxTextEngine, Bounds, Color, EdgeInsets, EventKind, EventPayload, MountBatch,
        MountItem, NodeId, Point, Points, PressEvent, ROOT_ID, Size, TextAlign, ViewKind,
    };
    use nsh_engine::{ColorScheme, Engine, EngineConfig, EngineControl, Environment, MountSink, Root};
    use serde::{Deserialize, Serialize};
    use tokio::net::TcpListener;

    const THEME_JSON: &str = include_str!("../../theme.json");

    struct ChannelSink {
        tx: Sender<MountBatch>,
    }

    impl MountSink for ChannelSink {
        fn submit(&self, batch: MountBatch) {
            let _ = self.tx.send(batch);
        }

        fn report_error(&self, message: &str) {
            eprintln!("[nsh_preview] NSH engine error: {message}");
        }
    }

    fn color_to_css(c: Color) -> Option<String> {
        let raw = c.0;
        let a = raw & 0xff;
        if a == 0 {
            return None;
        }
        let r = (raw >> 24) & 0xff;
        let g = (raw >> 16) & 0xff;
        let b = (raw >> 8) & 0xff;
        if a == 255 {
            Some(format!("#{r:02x}{g:02x}{b:02x}"))
        } else {
            Some(format!("rgba({r},{g},{b},{:.3})", a as f32 / 255.0))
        }
    }

    fn encode_base64(input: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
        for chunk in input.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = *chunk.get(1).unwrap_or(&0) as u32;
            let b2 = *chunk.get(2).unwrap_or(&0) as u32;
            let triple = (b0 << 16) | (b1 << 8) | b2;
            out.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
            out.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
            if chunk.len() > 1 {
                out.push(TABLE[((triple >> 6) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
            if chunk.len() > 2 {
                out.push(TABLE[(triple & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }

    fn image_source_to_browser_uri(source: &nsh_core::ImageSource) -> Option<String> {
        if let Some(data) = source.data.as_ref() {
            let mime = source
                .uri
                .strip_prefix("data:")
                .and_then(|rest| rest.split([';', ',']).next())
                .filter(|m| !m.is_empty())
                .unwrap_or("image/svg+xml");
            return Some(format!("data:{mime};base64,{}", encode_base64(data.bytes())));
        }
        if !source.uri.is_empty() && !source.uri.contains(";nsh-key=") {
            Some(source.uri.to_string())
        } else {
            None
        }
    }

    #[derive(Clone, Debug, Default, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SerializedBorderSide {
        width: f32,
        color: Option<String>,
    }

    #[derive(Clone, Debug, Default, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SerializedSpan {
        text: String,
        color: Option<String>,
        font_size: f32,
        font_weight: u16,
        italic: bool,
        tabular_nums: bool,
        line_height: f32,
    }

    #[derive(Clone, Debug, Default, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SerializedNode {
        id: u32,
        kind: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bg: Option<String>,
        radii: [f32; 4],
        borders: [SerializedBorderSide; 4],
        opacity: f32,
        backdrop_blur_px: f32,
        overflow_hidden: bool,
        text: Option<String>,
        spans: Vec<SerializedSpan>,
        text_align: String,
        max_lines: u32,
        content_x: f32,
        content_y: f32,
        content_w: f32,
        content_h: f32,
        image_uri: Option<String>,
        pressable: bool,
        scrollable: bool,
        scroll_horizontal: bool,
        scroll_content_w: f32,
        scroll_content_h: f32,
        scroll_x: f32,
        scroll_y: f32,
        is_input: bool,
        input_value: String,
        input_placeholder: String,
        input_color: Option<String>,
        input_placeholder_color: Option<String>,
        input_font_size: f32,
        test_id: Option<String>,
        a11y_label: Option<String>,
        pointer_none: bool,
        children: Vec<SerializedNode>,
    }

    #[derive(Clone, Debug)]
    struct NodeState {
        id: NodeId,
        kind: ViewKind,
        parent: Option<NodeId>,
        children: Vec<NodeId>,
        bounds: Bounds<Points>,
        bg: Option<String>,
        radii: [f32; 4],
        borders: [SerializedBorderSide; 4],
        opacity: f32,
        backdrop_blur_px: f32,
        overflow_hidden: bool,
        text: Option<String>,
        spans: Vec<SerializedSpan>,
        text_align: String,
        max_lines: u32,
        content_bounds: Bounds<Points>,
        image_uri: Option<String>,
        pressable: bool,
        scrollable: bool,
        scroll_horizontal: bool,
        scroll_content: Size<Points>,
        scroll_offset: Point<Points>,
        scroll_applied: u32,
        is_input: bool,
        input_value: String,
        input_count: u32,
        input_placeholder: String,
        input_color: Option<String>,
        input_placeholder_color: Option<String>,
        input_font_size: f32,
        test_id: Option<String>,
        a11y_label: Option<String>,
        pointer_none: bool,
    }

    impl NodeState {
        fn new(id: NodeId, kind: ViewKind) -> Self {
            Self {
                id,
                kind,
                parent: None,
                children: Vec::new(),
                bounds: Bounds::default(),
                bg: None,
                radii: [0.0; 4],
                borders: std::array::from_fn(|_| SerializedBorderSide::default()),
                opacity: 1.0,
                backdrop_blur_px: 0.0,
                overflow_hidden: false,
                text: None,
                spans: Vec::new(),
                text_align: "left".into(),
                max_lines: 0,
                content_bounds: Bounds::default(),
                image_uri: None,
                pressable: false,
                scrollable: matches!(kind, ViewKind::Scroll),
                scroll_horizontal: false,
                scroll_content: Size::default(),
                scroll_offset: Point::default(),
                scroll_applied: 0,
                is_input: matches!(kind, ViewKind::Input),
                input_value: String::new(),
                input_count: 0,
                input_placeholder: String::new(),
                input_color: None,
                input_placeholder_color: None,
                input_font_size: 14.0,
                test_id: None,
                a11y_label: None,
                pointer_none: false,
            }
        }
    }

    fn utf16_slice(s: &str, start: u32, end: u32) -> String {
        let mut idx = 0u32;
        let mut out = String::new();
        for ch in s.chars() {
            let len = ch.len_utf16() as u32;
            if idx >= start && idx + len <= end {
                out.push(ch);
            }
            idx += len;
        }
        out
    }

    struct NshScene {
        nodes: HashMap<NodeId, NodeState>,
        version: u64,
        device: String,
        scheme: String,
        width: f32,
        height: f32,
    }

    impl NshScene {
        fn new(width: f32, height: f32) -> Self {
            let mut nodes = HashMap::new();
            let mut root = NodeState::new(ROOT_ID, ViewKind::View);
            root.bounds = Bounds::new(
                Point::new(Points(0.0), Points(0.0)),
                Size::new(Points(width), Points(height)),
            );
            nodes.insert(ROOT_ID, root);
            Self {
                nodes,
                version: 0,
                device: "android".into(),
                scheme: "dark".into(),
                width,
                height,
            }
        }

        fn apply(&mut self, batch: MountBatch) {
            if batch.items.is_empty() {
                return;
            }
            self.version += 1;
            for item in batch.items {
                match item {
                    MountItem::Create { id, kind } => {
                        self.nodes.insert(id, NodeState::new(id, kind));
                    }
                    MountItem::Insert {
                        parent,
                        child,
                        before,
                    } => {
                        if let Some(prev_parent) =
                            self.nodes.get(&child).and_then(|n| n.parent)
                            && let Some(p) = self.nodes.get_mut(&prev_parent)
                        {
                            p.children.retain(|&c| c != child);
                        }
                        if let Some(c) = self.nodes.get_mut(&child) {
                            c.parent = Some(parent);
                        }
                        if let Some(p) = self.nodes.get_mut(&parent) {
                            p.children.retain(|&c| c != child);
                            if let Some(b) = before {
                                if let Some(pos) = p.children.iter().position(|&c| c == b) {
                                    p.children.insert(pos, child);
                                } else {
                                    p.children.push(child);
                                }
                            } else {
                                p.children.push(child);
                            }
                        }
                    }
                    MountItem::Remove { id } => {
                        if let Some(parent) = self.nodes.get_mut(&id).and_then(|n| n.parent.take())
                            && let Some(p) = self.nodes.get_mut(&parent)
                        {
                            p.children.retain(|&c| c != id);
                        }
                    }
                    MountItem::Delete { id } => {
                        if let Some(parent) = self.nodes.get(&id).and_then(|n| n.parent)
                            && let Some(p) = self.nodes.get_mut(&parent)
                        {
                            p.children.retain(|&c| c != id);
                        }
                        self.nodes.remove(&id);
                    }
                    MountItem::Frame { id, frame } => {
                        if let Some(n) = self.nodes.get_mut(&id) {
                            n.bounds = frame;
                        }
                    }
                    MountItem::ViewStyle { id, style } => {
                        if let Some(n) = self.nodes.get_mut(&id) {
                            n.bg = color_to_css(style.background_color);
                            let base_r = style.border_radius.0;
                            n.radii = [
                                style.corner_radius.top_left.map_or(base_r, |p| p.0),
                                style.corner_radius.top_right.map_or(base_r, |p| p.0),
                                style.corner_radius.bottom_right.map_or(base_r, |p| p.0),
                                style.corner_radius.bottom_left.map_or(base_r, |p| p.0),
                            ];
                            let w = style.borders.widths.to_array();
                            for (i, border_w) in w.iter().enumerate() {
                                n.borders[i] = SerializedBorderSide {
                                    width: border_w.0,
                                    color: color_to_css(style.borders.colors[i]),
                                };
                            }
                            n.opacity = style.opacity;
                            n.backdrop_blur_px = style
                                .effects
                                .as_ref()
                                .map(|e| nsh_core::blur_radius(&e.backdrop_filters).0)
                                .unwrap_or(0.0);
                            n.overflow_hidden = style.overflow_hidden;
                        }
                    }
                    MountItem::Text {
                        id, text, content, ..
                    } => {
                        if let Some(n) = self.nodes.get_mut(&id) {
                            n.text = Some(text.text.clone());
                            n.max_lines = text.max_lines;
                            n.text_align = match text.align {
                                TextAlign::Center => "center",
                                TextAlign::Right => "right",
                                TextAlign::Justify => "justify",
                                _ => "left",
                            }
                            .into();
                            n.content_bounds = content;
                            n.spans = text
                                .spans
                                .iter()
                                .map(|s| SerializedSpan {
                                    text: utf16_slice(&text.text, s.start, s.end),
                                    color: color_to_css(s.style.color),
                                    font_size: s.style.font_size.0,
                                    font_weight: s.style.font_weight,
                                    italic: s.style.italic,
                                    tabular_nums: s
                                        .style
                                        .font_variant
                                        .has(nsh_core::FontVariant::TABULAR_NUMS),
                                    line_height: s.style.line_height.0,
                                })
                                .collect();
                        }
                    }
                    MountItem::Image { id, source, .. } => {
                        if let Some(n) = self.nodes.get_mut(&id) {
                            n.image_uri = source.as_ref().and_then(image_source_to_browser_uri);
                        }
                    }
                    MountItem::Events { id, events } => {
                        if let Some(n) = self.nodes.get_mut(&id) {
                            n.pressable = events.contains(EventKind::Press)
                                || events.contains(EventKind::PressDown);
                        }
                    }
                    MountItem::ScrollConfig { id, horizontal, .. } => {
                        if let Some(n) = self.nodes.get_mut(&id) {
                            n.scrollable = true;
                            n.scroll_horizontal = horizontal;
                        }
                    }
                    MountItem::ScrollContent { id, size } => {
                        if let Some(n) = self.nodes.get_mut(&id) {
                            n.scroll_content = size;
                        }
                    }
                    MountItem::ViewProps { id, props } => {
                        if let Some(n) = self.nodes.get_mut(&id) {
                            n.test_id = props.test_id.as_ref().map(|s| s.to_string());
                            n.a11y_label = props.accessibility.label.as_ref().map(|s| s.to_string());
                            n.pointer_none =
                                matches!(props.pointer_events, nsh_core::PointerEvents::None);
                        }
                    }
                    MountItem::AccessibilityLabel { id, label } => {
                        if let Some(n) = self.nodes.get_mut(&id) {
                            n.a11y_label = label.as_ref().map(|s| s.to_string());
                        }
                    }
                    MountItem::Input { id, config } => {
                        if let Some(n) = self.nodes.get_mut(&id) {
                            n.is_input = true;
                            n.input_placeholder = config.placeholder.clone();
                            n.input_color = color_to_css(config.style.color);
                            n.input_placeholder_color =
                                config.placeholder_color.and_then(color_to_css);
                            n.input_font_size = config.style.font_size.0;
                        }
                    }
                    MountItem::InputValue { id, text, count } => {
                        if let Some(n) = self.nodes.get_mut(&id)
                            && count >= n.input_count
                        {
                            n.input_value = text;
                            n.input_count = count;
                        }
                    }
                    MountItem::InputContent { id, content } => {
                        if let Some(n) = self.nodes.get_mut(&id) {
                            n.content_bounds = content;
                        }
                    }
                    _ => {}
                }
            }
        }

        fn serialize_node(&self, id: NodeId) -> Option<SerializedNode> {
            let n = self.nodes.get(&id)?;
            let kind = match n.kind {
                ViewKind::View => "view",
                ViewKind::Text => "text",
                ViewKind::Scroll => "scroll",
                ViewKind::Input => "input",
                ViewKind::Image => "image",
                _ => "view",
            }
            .to_string();
            let children = n
                .children
                .iter()
                .filter_map(|&cid| self.serialize_node(cid))
                .collect();

            Some(SerializedNode {
                id: n.id,
                kind,
                x: n.bounds.origin.x.0,
                y: n.bounds.origin.y.0,
                w: n.bounds.size.width.0,
                h: n.bounds.size.height.0,
                bg: n.bg.clone(),
                radii: n.radii,
                borders: n.borders.clone(),
                opacity: n.opacity,
                backdrop_blur_px: n.backdrop_blur_px,
                overflow_hidden: n.overflow_hidden,
                text: n.text.clone(),
                spans: n.spans.clone(),
                text_align: n.text_align.clone(),
                max_lines: n.max_lines,
                content_x: n.content_bounds.origin.x.0,
                content_y: n.content_bounds.origin.y.0,
                content_w: n.content_bounds.size.width.0,
                content_h: n.content_bounds.size.height.0,
                image_uri: n.image_uri.clone(),
                pressable: n.pressable,
                scrollable: n.scrollable,
                scroll_horizontal: n.scroll_horizontal,
                scroll_content_w: n.scroll_content.width.0,
                scroll_content_h: n.scroll_content.height.0,
                scroll_x: n.scroll_offset.x.0,
                scroll_y: n.scroll_offset.y.0,
                is_input: n.is_input,
                input_value: n.input_value.clone(),
                input_placeholder: n.input_placeholder.clone(),
                input_color: n.input_color.clone(),
                input_placeholder_color: n.input_placeholder_color.clone(),
                input_font_size: n.input_font_size,
                test_id: n.test_id.clone(),
                a11y_label: n.a11y_label.clone(),
                pointer_none: n.pointer_none,
                children,
            })
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct TreeResponse {
        version: u64,
        engine: &'static str,
        device: String,
        scheme: String,
        width: f32,
        height: f32,
        apk_available: bool,
        apk_size_bytes: u64,
        root: Option<SerializedNode>,
    }

    struct HostState {
        _engine: Engine,
        control: EngineControl,
        rx: Receiver<MountBatch>,
        scene: NshScene,
    }

    impl HostState {
        fn drain_batches(&mut self, wait_first: Duration) {
            let deadline = Instant::now() + wait_first;
            let mut got_any = false;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let timeout = if got_any {
                    Duration::from_millis(8)
                } else {
                    remaining
                };
                match self.rx.recv_timeout(timeout) {
                    Ok(batch) => {
                        got_any = true;
                        self.scene.apply(batch);
                    }
                    Err(_) => break,
                }
            }
        }

        fn snapshot(&mut self) -> TreeResponse {
            self.drain_batches(Duration::from_millis(2));
            let apk_path = Path::new("public/resonance-arm64-debug.apk");
            let apk_meta = std::fs::metadata(apk_path).ok();
            TreeResponse {
                version: self.scene.version,
                engine: "Pure Rust NSH 0.7.0 (nsh-engine + nsh-core + nsh-base)",
                device: self.scene.device.clone(),
                scheme: self.scene.scheme.clone(),
                width: self.scene.width,
                height: self.scene.height,
                apk_available: apk_meta.is_some(),
                apk_size_bytes: apk_meta.map(|m| m.len()).unwrap_or(0),
                root: self.scene.serialize_node(ROOT_ID),
            }
        }
    }

    #[derive(Deserialize)]
    struct PressReq {
        id: u32,
    }

    #[derive(Deserialize)]
    struct InputReq {
        id: u32,
        text: String,
    }

    #[derive(Deserialize)]
    struct ScrollReq {
        id: u32,
        x: f32,
        y: f32,
    }

    #[derive(Deserialize)]
    struct ConfigReq {
        device: Option<String>,
        scheme: Option<String>,
    }

    fn json_response<T: Serialize>(status: StatusCode, body: &T) -> Response<Full<Bytes>> {
        let bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "content-type")
            .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, POST, OPTIONS")
            .body(Full::new(Bytes::from(bytes)))
            .unwrap()
    }

    fn make_environment(device: &str, scheme: &str) -> (Size<Points>, Environment) {
        let color_scheme = if scheme == "light" {
            ColorScheme::Light
        } else {
            ColorScheme::Dark
        };
        if device == "desktop" {
            (
                Size::new(Points(1180.0), Points(780.0)),
                Environment {
                    color_scheme,
                    os: Os::Linux,
                    safe_area: EdgeInsets::default(),
                    ..Environment::default()
                },
            )
        } else {
            (
                Size::new(Points(390.0), Points(844.0)),
                Environment {
                    color_scheme,
                    os: Os::Android,
                    safe_area: EdgeInsets {
                        top: Points(24.0),
                        right: Points(0.0),
                        bottom: Points(14.0),
                        left: Points(0.0),
                    },
                    ..Environment::default()
                },
            )
        }
    }

    async fn handle_request(
        req: Request<hyper::body::Incoming>,
        state: Arc<Mutex<HostState>>,
    ) -> Result<Response<Full<Bytes>>, Infallible> {
        if req.method() == Method::OPTIONS {
            return Ok(json_response(StatusCode::NO_CONTENT, &serde_json::json!({})));
        }
        let path = req.uri().path().to_string();
        let method = req.method().clone();
        let body_bytes = req
            .into_body()
            .collect()
            .await
            .map(|b| b.to_bytes())
            .unwrap_or_default();

        let resp = match (method, path.as_str()) {
            (Method::GET, "/api/nsh/tree") => {
                let mut st = state.lock().unwrap();
                json_response(StatusCode::OK, &st.snapshot())
            }
            (Method::POST, "/api/nsh/press") => {
                if let Ok(p) = serde_json::from_slice::<PressReq>(&body_bytes) {
                    let mut st = state.lock().unwrap();
                    let payload = EventPayload::Press(PressEvent {
                        position: Point::new(Points(8.0), Points(8.0)),
                        page_position: Point::new(Points(8.0), Points(8.0)),
                        modifiers: Default::default(),
                        clicks: 1,
                    });
                    st.control
                        .dispatch_event(p.id, EventKind::PressDown, payload.clone());
                    st.control
                        .dispatch_event(p.id, EventKind::PressUp, payload.clone());
                    st.control.dispatch_event(p.id, EventKind::Press, payload);
                    st.drain_batches(Duration::from_millis(80));
                    json_response(StatusCode::OK, &st.snapshot())
                } else {
                    json_response(
                        StatusCode::BAD_REQUEST,
                        &serde_json::json!({"error": "invalid press payload"}),
                    )
                }
            }
            (Method::POST, "/api/nsh/input") => {
                if let Ok(inp) = serde_json::from_slice::<InputReq>(&body_bytes) {
                    let mut st = state.lock().unwrap();
                    let count = if let Some(n) = st.scene.nodes.get_mut(&inp.id) {
                        n.input_count += 1;
                        n.input_value = inp.text.clone();
                        n.input_count
                    } else {
                        1
                    };
                    st.control.dispatch_event(
                        inp.id,
                        EventKind::ChangeText,
                        EventPayload::Text {
                            text: inp.text,
                            count,
                        },
                    );
                    st.drain_batches(Duration::from_millis(80));
                    json_response(StatusCode::OK, &st.snapshot())
                } else {
                    json_response(
                        StatusCode::BAD_REQUEST,
                        &serde_json::json!({"error": "invalid input payload"}),
                    )
                }
            }
            (Method::POST, "/api/nsh/submit") => {
                if let Ok(inp) = serde_json::from_slice::<InputReq>(&body_bytes) {
                    let mut st = state.lock().unwrap();
                    let count = st
                        .scene
                        .nodes
                        .get(&inp.id)
                        .map(|n| n.input_count)
                        .unwrap_or(1);
                    st.control.dispatch_event(
                        inp.id,
                        EventKind::SubmitEditing,
                        EventPayload::Text {
                            text: inp.text,
                            count,
                        },
                    );
                    st.drain_batches(Duration::from_millis(80));
                    json_response(StatusCode::OK, &st.snapshot())
                } else {
                    json_response(
                        StatusCode::BAD_REQUEST,
                        &serde_json::json!({"error": "invalid submit payload"}),
                    )
                }
            }
            (Method::POST, "/api/nsh/scroll") => {
                if let Ok(sc) = serde_json::from_slice::<ScrollReq>(&body_bytes) {
                    let mut st = state.lock().unwrap();
                    let applied = if let Some(n) = st.scene.nodes.get_mut(&sc.id) {
                        n.scroll_offset = Point::new(Points(sc.x), Points(sc.y));
                        n.scroll_applied += 1;
                        n.scroll_applied
                    } else {
                        1
                    };
                    st.control
                        .scroll(sc.id, Point::new(Points(sc.x), Points(sc.y)), applied);
                    st.drain_batches(Duration::from_millis(40));
                    json_response(StatusCode::OK, &st.snapshot())
                } else {
                    json_response(
                        StatusCode::BAD_REQUEST,
                        &serde_json::json!({"error": "invalid scroll payload"}),
                    )
                }
            }
            (Method::POST, "/api/nsh/config") => {
                if let Ok(cfg) = serde_json::from_slice::<ConfigReq>(&body_bytes) {
                    let mut st = state.lock().unwrap();
                    if let Some(d) = cfg.device {
                        st.scene.device = d;
                    }
                    if let Some(s) = cfg.scheme {
                        st.scene.scheme = s;
                    }
                    let (size, env) = make_environment(&st.scene.device, &st.scene.scheme);
                    st.scene.width = size.width.0;
                    st.scene.height = size.height.0;
                    if let Some(root) = st.scene.nodes.get_mut(&ROOT_ID) {
                        root.bounds = Bounds::new(Point::new(Points(0.0), Points(0.0)), size);
                    }
                    st.control.set_environment(env);
                    st.control.resize(size);
                    st.drain_batches(Duration::from_millis(100));
                    json_response(StatusCode::OK, &st.snapshot())
                } else {
                    json_response(
                        StatusCode::BAD_REQUEST,
                        &serde_json::json!({"error": "invalid config payload"}),
                    )
                }
            }
            _ => json_response(StatusCode::NOT_FOUND, &serde_json::json!({"error": "not found"})),
        };
        Ok(resp)
    }

    pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
        // Seed deterministic QA dataset in Pure Rust store
        resonance::store::enable_fixture_mode(true);

        let (tx, rx) = crossbeam_channel::unbounded();
        let (initial_size, initial_env) = make_environment("android", "dark");
        let tokens = nsh_base::tokens_of(Some(THEME_JSON))
            .expect("theme.json must parse into valid NSH tokens");

        let config = EngineConfig {
            viewport: initial_size,
            environment: initial_env,
            ..EngineConfig::new(Root::of::<resonance::App>())
        };

        let engine = Engine::start_with(
            config,
            ApproxTextEngine::default,
            ChannelSink { tx },
            move |ctrl| {
                ctrl.set_tokens(Some(tokens));
            },
        )
        .expect("Pure Rust NSH engine starts cleanly");

        let control = engine.control();
        let mut host = HostState {
            _engine: engine,
            control,
            rx,
            scene: NshScene::new(initial_size.width.0, initial_size.height.0),
        };
        // Wait for initial NSH mount commit
        host.drain_batches(Duration::from_millis(250));

        let shared = Arc::new(Mutex::new(host));
        let port: u16 = std::env::var("NSH_PREVIEW_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3001);
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = TcpListener::bind(addr).await?;
        println!("Resonance Pure Rust NSH Preview Engine listening on http://{addr}");

        loop {
            let (stream, _) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let st = Arc::clone(&shared);
            tokio::spawn(async move {
                let svc = service_fn(move |req| handle_request(req, Arc::clone(&st)));
                let _ = http1::Builder::new().serve_connection(io, svc).await;
            });
        }
    }
}

#[cfg(not(target_os = "android"))]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    host::run().await
}
