//! A deliberately bounded LaTeX subset for editable Office Math.
//! Supports groups, fractions, square/indexed roots, scripts, literal symbols,
//! Greek/operator commands, text runs and matrix/pmatrix/bmatrix environments.
//! Unsupported syntax is an error: exporters must retain the original LaTeX.
use crate::types::*;
const M: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";
const MAX_NODES: usize = 4096;
const MAX_DEPTH: usize = 32;
#[derive(Clone, Debug)]
enum Node {
    Run(String, bool),
    Group(Vec<Node>),
    Fraction(Vec<Node>, Vec<Node>),
    Root(Option<Vec<Node>>, Vec<Node>),
    Script(Box<Node>, Option<Vec<Node>>, Option<Vec<Node>>),
    Matrix(String, Vec<Vec<Vec<Node>>>),
}
#[derive(Clone, Debug)]
pub struct Rendered {
    pub omml: String,
    pub canonical: String,
}
fn error(message: &str) -> anyhow::Error {
    Fault::new("NG201", format!("Equation retained as LaTeX: {message}")).into()
}
struct Parser<'a> {
    input: &'a str,
    pos: usize,
    nodes: usize,
}
impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }
    fn next(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }
    fn ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.next();
        }
    }
    fn counted(&mut self, node: Node) -> Result<Node> {
        self.nodes += 1;
        anyhow::ensure!(
            self.nodes <= MAX_NODES,
            error("expression exceeds 4,096 nodes")
        );
        Ok(node)
    }
    fn raw_group(&mut self) -> Result<String> {
        self.ws();
        anyhow::ensure!(
            self.next() == Some('{'),
            error("expected a braced argument")
        );
        let mut text = String::new();
        let mut depth = 1;
        while let Some(ch) = self.next() {
            match ch {
                '{' => {
                    depth += 1;
                    anyhow::ensure!(depth <= MAX_DEPTH, error("group nesting exceeds 32"));
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(text);
                    }
                }
                '\\' => {
                    if self.input[self.pos..].starts_with("textbackslash{}") {
                        self.pos += "textbackslash{}".len();
                        text.push('\\');
                        continue;
                    }
                    let escaped = self.next().ok_or_else(|| error("trailing escape"))?;
                    anyhow::ensure!(
                        ['{', '}', '&', '%', '$', '_', '#', ' '].contains(&escaped),
                        error("unsupported escape in text argument")
                    );
                    text.push(escaped);
                }
                _ => {
                    anyhow::ensure!(
                        !ch.is_control(),
                        error("multiline/control text arguments are unsupported")
                    );
                    text.push(ch);
                }
            }
        }
        Err(error("unclosed braced argument"))
    }
    fn argument(&mut self, depth: usize) -> Result<Vec<Node>> {
        self.ws();
        if self.peek() == Some('{') {
            self.next();
            let (items, end) = self.expression(depth + 1, false)?;
            anyhow::ensure!(end == End::Group, error("unclosed argument"));
            Ok(items)
        } else {
            Ok(vec![self.atom(depth + 1)?])
        }
    }
    fn command(&mut self, depth: usize) -> Result<Node> {
        let mut name = String::new();
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            name.push(self.next().unwrap_or_default());
        }
        if name.is_empty() {
            let symbol = self.next().ok_or_else(|| error("trailing backslash"))?;
            if [',', ';', '!'].contains(&symbol) {
                return self.counted(Node::Run(String::new(), false));
            }
            anyhow::ensure!(
                ['{', '}', '_', '%', '$', '#', '&', ' '].contains(&symbol),
                error("unsupported escaped symbol")
            );
            return self.counted(Node::Run(symbol.to_string(), false));
        }
        match name.as_str() {
            "frac" | "dfrac" | "tfrac" => {
                let numerator = self.argument(depth)?;
                let denominator = self.argument(depth)?;
                self.counted(Node::Fraction(numerator, denominator))
            }
            "sqrt" => {
                self.ws();
                let degree = if self.peek() == Some('[') {
                    self.next();
                    let start = self.pos;
                    let mut nested = 0usize;
                    while let Some(ch) = self.peek() {
                        if ch == ']' && nested == 0 {
                            break;
                        }
                        if ch == '{' {
                            nested += 1;
                        }
                        if ch == '}' {
                            nested = nested.saturating_sub(1);
                        }
                        self.next();
                    }
                    anyhow::ensure!(self.peek() == Some(']'), error("unclosed root index"));
                    let source = &self.input[start..self.pos];
                    self.next();
                    let mut index = Parser {
                        input: source,
                        pos: 0,
                        nodes: 0,
                    };
                    let (nodes, end) = index.expression(depth + 1, false)?;
                    anyhow::ensure!(end == End::Input, error("invalid root index"));
                    self.nodes += index.nodes;
                    (!nodes.is_empty()).then_some(nodes)
                } else {
                    None
                };
                let body = self.argument(depth)?;
                self.counted(Node::Root(degree, body))
            }
            "text" => {
                let text = self.raw_group()?;
                self.counted(Node::Run(text, true))
            }
            "begin" => {
                let environment = self.raw_group()?;
                anyhow::ensure!(
                    ["matrix", "pmatrix", "bmatrix"].contains(&environment.as_str()),
                    error("only matrix, pmatrix and bmatrix environments are supported")
                );
                let mut rows = vec![];
                let mut row = vec![];
                loop {
                    let (cell, end) = self.expression(depth + 1, true)?;
                    row.push(cell);
                    anyhow::ensure!(row.len() <= 20, error("matrix exceeds 20 columns"));
                    match end {
                        End::Column => {}
                        End::Row => {
                            rows.push(row);
                            row = vec![];
                        }
                        End::Matrix(ref closing) if closing == &environment => {
                            rows.push(row);
                            break;
                        }
                        _ => return Err(error("unclosed or mismatched matrix environment")),
                    }
                    anyhow::ensure!(rows.len() < 20, error("matrix exceeds 20 rows"));
                }
                let cols = rows.first().map_or(0, Vec::len);
                anyhow::ensure!(
                    cols > 0 && rows.iter().all(|r| r.len() == cols),
                    error("matrix rows have different lengths")
                );
                self.counted(Node::Matrix(environment, rows))
            }
            "left" | "right" => Err(error(
                "scalable left/right delimiters are unsupported; use literal parentheses or a bracketed matrix",
            )),
            "," | ";" | "!" | "quad" | "qquad" => self.counted(Node::Run(String::new(), false)),
            _ => {
                let symbol = match name.as_str() {
                    "alpha" => "α",
                    "beta" => "β",
                    "gamma" => "γ",
                    "delta" => "δ",
                    "epsilon" => "ϵ",
                    "varepsilon" => "ε",
                    "zeta" => "ζ",
                    "eta" => "η",
                    "theta" => "θ",
                    "vartheta" => "ϑ",
                    "iota" => "ι",
                    "kappa" => "κ",
                    "lambda" => "λ",
                    "mu" => "μ",
                    "nu" => "ν",
                    "xi" => "ξ",
                    "pi" => "π",
                    "rho" => "ρ",
                    "sigma" => "σ",
                    "tau" => "τ",
                    "upsilon" => "υ",
                    "phi" => "ϕ",
                    "varphi" => "φ",
                    "chi" => "χ",
                    "psi" => "ψ",
                    "omega" => "ω",
                    "Gamma" => "Γ",
                    "Delta" => "Δ",
                    "Theta" => "Θ",
                    "Lambda" => "Λ",
                    "Xi" => "Ξ",
                    "Pi" => "Π",
                    "Sigma" => "Σ",
                    "Upsilon" => "Υ",
                    "Phi" => "Φ",
                    "Psi" => "Ψ",
                    "Omega" => "Ω",
                    "times" => "×",
                    "cdot" => "·",
                    "div" => "÷",
                    "pm" => "±",
                    "mp" => "∓",
                    "le" | "leq" => "≤",
                    "ge" | "geq" => "≥",
                    "ne" | "neq" => "≠",
                    "approx" => "≈",
                    "equiv" => "≡",
                    "infty" => "∞",
                    "sum" => "∑",
                    "prod" => "∏",
                    "int" => "∫",
                    "partial" => "∂",
                    "nabla" => "∇",
                    "in" => "∈",
                    "notin" => "∉",
                    "subset" => "⊂",
                    "subseteq" => "⊆",
                    "cup" => "∪",
                    "cap" => "∩",
                    "to" | "rightarrow" => "→",
                    "leftarrow" => "←",
                    "leftrightarrow" => "↔",
                    "forall" => "∀",
                    "exists" => "∃",
                    "emptyset" => "∅",
                    "backslash" => "\\",
                    "sin" => "sin",
                    "cos" => "cos",
                    "tan" => "tan",
                    "log" => "log",
                    "ln" => "ln",
                    "lim" => "lim",
                    _ => return Err(error(&format!("unsupported command \\{name}"))),
                };
                self.counted(Node::Run(
                    symbol.into(),
                    ["sin", "cos", "tan", "log", "ln", "lim"].contains(&name.as_str()),
                ))
            }
        }
    }
    fn atom(&mut self, depth: usize) -> Result<Node> {
        anyhow::ensure!(depth <= MAX_DEPTH, error("nesting exceeds 32"));
        self.ws();
        match self.next().ok_or_else(|| error("missing argument"))? {
            '{' => {
                let (nodes, end) = self.expression(depth + 1, false)?;
                anyhow::ensure!(end == End::Group, error("unclosed group"));
                self.counted(Node::Group(nodes))
            }
            '\\' => self.command(depth),
            '_' | '^' | '}' | '&' | '$' => {
                Err(error("unexpected script, group or alignment marker"))
            }
            ch if ch.is_control() => Err(error("unsupported control character")),
            ch => self.counted(Node::Run(ch.to_string(), false)),
        }
    }
    fn expression(&mut self, depth: usize, matrix: bool) -> Result<(Vec<Node>, End)> {
        anyhow::ensure!(depth <= MAX_DEPTH, error("nesting exceeds 32"));
        let mut out = vec![];
        loop {
            self.ws();
            let Some(ch) = self.peek() else {
                return Ok((out, End::Input));
            };
            if ch == '}' {
                self.next();
                return Ok((out, End::Group));
            }
            if matrix && ch == '&' {
                self.next();
                return Ok((out, End::Column));
            }
            if matrix && self.input[self.pos..].starts_with("\\\\") {
                self.pos += 2;
                return Ok((out, End::Row));
            }
            if matrix && self.input[self.pos..].starts_with("\\end") {
                self.pos += 4;
                let end = self.raw_group()?;
                return Ok((out, End::Matrix(end)));
            }
            if ch == '_' || ch == '^' {
                self.next();
                let value = self.argument(depth)?;
                let previous = out.pop().ok_or_else(|| error("script has no base"))?;
                let (base, mut sub, mut sup) = match previous {
                    Node::Script(base, sub, sup) => (base, sub, sup),
                    other => (Box::new(other), None, None),
                };
                if ch == '_' {
                    anyhow::ensure!(sub.is_none(), error("duplicate subscript"));
                    sub = Some(value);
                } else {
                    anyhow::ensure!(sup.is_none(), error("duplicate superscript"));
                    sup = Some(value);
                }
                out.push(self.counted(Node::Script(base, sub, sup))?);
                continue;
            }
            let atom = self.atom(depth)?;
            if !matches!(&atom,Node::Run(text,_)if text.is_empty()) {
                out.push(atom);
            }
        }
    }
}
#[derive(PartialEq)]
enum End {
    Input,
    Group,
    Column,
    Row,
    Matrix(String),
}
fn strip(input: &str) -> &str {
    let input = input.trim();
    for (start, end) in [("$$", "$$"), ("\\[", "\\]"), ("\\(", "\\)"), ("$", "$")] {
        if let Some(inner) = input.strip_prefix(start).and_then(|v| v.strip_suffix(end)) {
            return inner;
        }
    }
    input
}
fn parse(input: &str) -> Result<Vec<Node>> {
    anyhow::ensure!(input.len() <= 16000, error("source exceeds 16,000 bytes"));
    let source = strip(input);
    let mut parser = Parser {
        input: source,
        pos: 0,
        nodes: 0,
    };
    let (nodes, end) = parser.expression(0, false)?;
    anyhow::ensure!(
        end == End::Input && parser.pos == source.len() && !nodes.is_empty(),
        error("empty or unbalanced expression")
    );
    Ok(nodes)
}
fn escaped_text(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if ch == '\\' {
            out.push_str("\\textbackslash{}");
            continue;
        }
        if ['{', '}', '&', '%', '$', '_', '#'].contains(&ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}
fn canonical_nodes(nodes: &[Node]) -> String {
    let mut out = String::new();
    let mut normal = String::new();
    for node in nodes {
        if let Node::Run(text, true) = node {
            normal.push_str(text);
            continue;
        }
        if !normal.is_empty() {
            out.push_str(&format!("\\text{{{}}}", escaped_text(&normal)));
            normal.clear();
        }
        if let Node::Group(body) = node {
            out.push_str(&canonical_nodes(body));
        } else {
            out.push_str(&canonical_node(node));
        }
    }
    if !normal.is_empty() {
        out.push_str(&format!("\\text{{{}}}", escaped_text(&normal)));
    }
    out
}
fn canonical_node(node: &Node) -> String {
    match node {
        Node::Run(text, normal) => {
            if *normal {
                format!("\\text{{{}}}", escaped_text(text))
            } else {
                text.chars()
                    .map(|c| {
                        if c == '\\' {
                            "\\backslash ".into()
                        } else {
                            escaped_text(&c.to_string())
                        }
                    })
                    .collect()
            }
        }
        Node::Group(body) => format!("{{{}}}", canonical_nodes(body)),
        Node::Fraction(num, den) => format!(
            "\\frac{{{}}}{{{}}}",
            canonical_nodes(num),
            canonical_nodes(den)
        ),
        Node::Root(degree, body) => format!(
            "\\sqrt{}{{{}}}",
            degree
                .as_ref()
                .map(|n| format!("[{}]", canonical_nodes(n)))
                .unwrap_or_default(),
            canonical_nodes(body)
        ),
        Node::Script(base, sub, sup) => {
            let mut result = match base.as_ref() {
                Node::Run(text, false) if text.chars().count() > 1 => {
                    format!("{{{}}}", canonical_node(base))
                }
                _ => canonical_node(base),
            };
            if let Some(sub) = sub {
                result.push_str(&format!("_{{{}}}", canonical_nodes(sub)));
            }
            if let Some(sup) = sup {
                result.push_str(&format!("^{{{}}}", canonical_nodes(sup)));
            }
            result
        }
        Node::Matrix(environment, rows) => format!(
            "\\begin{{{environment}}}{}\\end{{{environment}}}",
            rows.iter()
                .map(|row| row
                    .iter()
                    .map(|cell| canonical_nodes(cell))
                    .collect::<Vec<_>>()
                    .join("&"))
                .collect::<Vec<_>>()
                .join("\\\\")
        ),
    }
}
fn omml_nodes(nodes: &[Node]) -> String {
    nodes.iter().map(omml_node).collect()
}
fn omml_node(node: &Node) -> String {
    match node {
        Node::Run(text, normal) => format!(
            "<m:r>{}<m:t xml:space=\"preserve\">{}</m:t></m:r>",
            if *normal {
                "<m:rPr><m:nor/></m:rPr>"
            } else {
                ""
            },
            crate::word::xml(text)
        ),
        Node::Group(body) => format!("<m:box><m:e>{}</m:e></m:box>", omml_nodes(body)),
        Node::Fraction(num, den) => format!(
            "<m:f><m:num>{}</m:num><m:den>{}</m:den></m:f>",
            omml_nodes(num),
            omml_nodes(den)
        ),
        Node::Root(degree, body) => format!(
            "<m:rad><m:radPr><m:degHide m:val=\"{}\"/></m:radPr><m:deg>{}</m:deg><m:e>{}</m:e></m:rad>",
            if degree.is_none() { 1 } else { 0 },
            degree.as_ref().map(|n| omml_nodes(n)).unwrap_or_default(),
            omml_nodes(body)
        ),
        Node::Script(base, sub, sup) => {
            let name = if sub.is_some() && sup.is_some() {
                "sSubSup"
            } else if sub.is_some() {
                "sSub"
            } else {
                "sSup"
            };
            format!(
                "<m:{name}><m:e>{}</m:e>{}{}</m:{name}>",
                omml_node(base),
                sub.as_ref()
                    .map(|n| format!("<m:sub>{}</m:sub>", omml_nodes(n)))
                    .unwrap_or_default(),
                sup.as_ref()
                    .map(|n| format!("<m:sup>{}</m:sup>", omml_nodes(n)))
                    .unwrap_or_default()
            )
        }
        Node::Matrix(environment, rows) => {
            let body = format!(
                "<m:m>{}</m:m>",
                rows.iter()
                    .map(|row| format!(
                        "<m:mr>{}</m:mr>",
                        row.iter()
                            .map(|cell| format!("<m:e>{}</m:e>", omml_nodes(cell)))
                            .collect::<String>()
                    ))
                    .collect::<String>()
            );
            match environment.as_str() {
                "pmatrix" => format!(
                    "<m:d><m:dPr><m:begChr m:val=\"(\"/><m:endChr m:val=\")\"/></m:dPr><m:e>{body}</m:e></m:d>"
                ),
                "bmatrix" => format!(
                    "<m:d><m:dPr><m:begChr m:val=\"[\"/><m:endChr m:val=\"]\"/></m:dPr><m:e>{body}</m:e></m:d>"
                ),
                _ => body,
            }
        }
    }
}
pub fn canonical(input: &str) -> Result<String> {
    Ok(canonical_nodes(&parse(input)?))
}
pub fn convert(input: &str) -> Result<Rendered> {
    let nodes = parse(input)?;
    Ok(Rendered {
        omml: format!("<m:oMath xmlns:m=\"{M}\">{}</m:oMath>", omml_nodes(&nodes)),
        canonical: canonical_nodes(&nodes),
    })
}
fn math(n: roxmltree::Node<'_, '_>, name: &str) -> bool {
    n.tag_name().namespace() == Some(M) && n.tag_name().name() == name
}
fn children<'a, 'input>(
    n: roxmltree::Node<'a, 'input>,
) -> impl Iterator<Item = roxmltree::Node<'a, 'input>> {
    n.children().filter(|n| n.is_element())
}
fn part<'a, 'b>(n: roxmltree::Node<'a, 'b>, name: &str) -> Result<roxmltree::Node<'a, 'b>> {
    let values = children(n).filter(|n| math(*n, name)).collect::<Vec<_>>();
    anyhow::ensure!(
        values.len() == 1,
        error("Word equation has a missing or repeated argument")
    );
    Ok(values[0])
}
fn from_nodes(n: roxmltree::Node<'_, '_>, depth: usize, count: &mut usize) -> Result<Vec<Node>> {
    children(n)
        .filter(|n| !math(*n, "argPr") && !math(*n, "ctrlPr"))
        .map(|n| from_node(n, depth + 1, count))
        .collect()
}
fn only(n: roxmltree::Node<'_, '_>, allowed: &[&str]) -> Result<()> {
    anyhow::ensure!(
        children(n).all(|child| child.tag_name().namespace() == Some(M)
            && (allowed.contains(&child.tag_name().name()) || math(child, "ctrlPr"))),
        error("unsupported Word equation structure")
    );
    Ok(())
}
fn from_node(n: roxmltree::Node<'_, '_>, depth: usize, count: &mut usize) -> Result<Node> {
    *count += 1;
    anyhow::ensure!(
        depth <= MAX_DEPTH && *count <= MAX_NODES && n.tag_name().namespace() == Some(M),
        error("Word equation exceeds limits or has an unknown namespace")
    );
    match n.tag_name().name() {
        "r" => {
            anyhow::ensure!(
                children(n).all(|n| math(n, "t")
                    || math(n, "rPr")
                    || n.tag_name().namespace()
                        == Some("http://schemas.openxmlformats.org/wordprocessingml/2006/main")
                        && n.tag_name().name() == "rPr"),
                error("unsupported Word math run")
            );
            let text = children(n)
                .filter(|n| math(*n, "t"))
                .filter_map(|n| n.text())
                .collect::<String>();
            let normal = n.descendants().any(|n| {
                math(n, "nor")
                    && n.attribute((M, "val"))
                        .is_none_or(|v| !["0", "false", "off"].contains(&v))
            });
            Ok(Node::Run(text, normal))
        }
        "box" => {
            only(n, &["boxPr", "e"])?;
            Ok(Node::Group(from_nodes(part(n, "e")?, depth, count)?))
        }
        "f" => {
            only(n, &["fPr", "num", "den"])?;
            if let Some(style) = n.descendants().find(|n| math(*n, "type")) {
                anyhow::ensure!(
                    style.attribute((M, "val")).is_none_or(|v| v == "bar"),
                    error("unsupported Word fraction style")
                );
            }
            Ok(Node::Fraction(
                from_nodes(part(n, "num")?, depth, count)?,
                from_nodes(part(n, "den")?, depth, count)?,
            ))
        }
        "rad" => {
            only(n, &["radPr", "deg", "e"])?;
            let degree = from_nodes(part(n, "deg")?, depth, count)?;
            if children(n)
                .find(|child| math(*child, "radPr"))
                .is_some_and(|properties| {
                    children(properties).any(|child| {
                        math(child, "degHide")
                            && child
                                .attribute((M, "val"))
                                .is_none_or(|v| ["1", "true", "on"].contains(&v))
                    })
                })
            {
                anyhow::ensure!(
                    degree.is_empty(),
                    error("a hidden nonempty root degree is unsupported")
                );
            }
            Ok(Node::Root(
                (!degree.is_empty()).then_some(degree),
                from_nodes(part(n, "e")?, depth, count)?,
            ))
        }
        "sSub" | "sSup" | "sSubSup" => {
            only(
                n,
                match n.tag_name().name() {
                    "sSub" => &["sSubPr", "e", "sub"],
                    "sSup" => &["sSupPr", "e", "sup"],
                    _ => &["sSubSupPr", "e", "sub", "sup"],
                },
            )?;
            let base = from_nodes(part(n, "e")?, depth, count)?;
            let base = if base.len() == 1 {
                base.into_iter()
                    .next()
                    .ok_or_else(|| error("missing script base"))?
            } else {
                Node::Group(base)
            };
            let sub = if n.tag_name().name() != "sSup" {
                Some(from_nodes(part(n, "sub")?, depth, count)?)
            } else {
                None
            };
            let sup = if n.tag_name().name() != "sSub" {
                Some(from_nodes(part(n, "sup")?, depth, count)?)
            } else {
                None
            };
            Ok(Node::Script(Box::new(base), sub, sup))
        }
        "m" => {
            let mut rows = vec![];
            for row in children(n).filter(|r| !math(*r, "mPr")) {
                anyhow::ensure!(
                    math(row, "mr") && rows.len() < 20,
                    error("unsupported Word matrix row")
                );
                let mut cells = vec![];
                for cell in children(row) {
                    anyhow::ensure!(
                        math(cell, "e") && cells.len() < 20,
                        error("unsupported Word matrix cell")
                    );
                    cells.push(from_nodes(cell, depth, count)?);
                }
                rows.push(cells);
            }
            let cols = rows.first().map_or(0, Vec::len);
            anyhow::ensure!(
                cols > 0 && rows.iter().all(|r| r.len() == cols),
                error("invalid Word matrix dimensions")
            );
            Ok(Node::Matrix("matrix".into(), rows))
        }
        "d" => {
            only(n, &["dPr", "e"])?;
            let properties = part(n, "dPr")?;
            let begin = part(properties, "begChr")?
                .attribute((M, "val"))
                .unwrap_or("");
            let end = part(properties, "endChr")?
                .attribute((M, "val"))
                .unwrap_or("");
            let environment = match (begin, end) {
                ("(", ")") => "pmatrix",
                ("[", "]") => "bmatrix",
                _ => return Err(error("unsupported Word matrix delimiters")),
            };
            let expression = from_nodes(part(n, "e")?, depth, count)?;
            anyhow::ensure!(
                expression.len() == 1,
                error("unsupported delimited Word equation")
            );
            match expression.into_iter().next() {
                Some(Node::Matrix(_, rows)) => Ok(Node::Matrix(environment.into(), rows)),
                _ => Err(error("delimited Word equation is not a supported matrix")),
            }
        }
        _ => Err(error(&format!(
            "unsupported Word equation element {}",
            n.tag_name().name()
        ))),
    }
}
/// Convert only the supported Office Math structures back to canonical source LaTeX.
pub fn from_omml(node: roxmltree::Node<'_, '_>) -> Result<String> {
    anyhow::ensure!(
        math(node, "oMath"),
        error("expected an Office Math equation")
    );
    let nodes = from_nodes(node, 0, &mut 0)?;
    anyhow::ensure!(!nodes.is_empty(), error("empty Office Math equation"));
    let result = canonical_nodes(&nodes);
    anyhow::ensure!(
        !result.trim().is_empty() && result.len() <= 16000,
        error("Word equation source exceeds 16,000 bytes")
    );
    Ok(result)
}
