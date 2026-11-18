//! A flow as an Excalidraw drawing, so a flow can be made and changed in
//! the embedded drawing tab. This module only reads the scene file; the
//! drawing page has no way into the app.
//!
//! - A box (rectangle or ellipse) with text is a command step. Line 1 is
//!   the step ID, line 2 is the command, the next lines are the prompt.
//!   A line `cwd: FOLDER` or `worktree: NAME` is an option.
//! - A diamond with text is a gate. Line 1 is the ID, the next lines are
//!   the question.
//! - An arrow from A to B means that B needs A.
//! - All other shapes and text are notes.
//!
//! The result goes through the same checks as a TOML flow file.
use super::{Flow, FlowError, RawStep, build};
use serde_json::{Value, json};
use std::collections::HashMap;

/// File extension of a drawn flow.
pub const EXTENSION: &str = "excalidraw";

fn error(text: impl Into<String>) -> FlowError {
    FlowError::Drawing(text.into())
}

fn text_of<'a>(element: &'a Value, key: &str) -> Option<&'a str> {
    element.get(key).and_then(Value::as_str)
}

fn number(element: &Value, key: &str) -> f64 {
    element.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

/// Height of one row of shapes for the step order.
const ROW: f64 = 100.0;

/// The row and the centre x of a shape: shapes side by side are one row,
/// also when their heights are different.
fn place(element: &Value) -> (f64, f64) {
    let centre =
        |start: &str, size: &str| number(element, size).mul_add(0.5, number(element, start));
    ((centre("y", "height") / ROW).floor(), centre("x", "width"))
}

/// One shape with text: its position and the step it gives.
struct Shape {
    place: (f64, f64),
    step: RawStep,
}

/// The step of one shape's text. `gate` is true for a diamond.
fn step_of(text: &str, gate: bool) -> Result<RawStep, FlowError> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .skip_while(|line| line.is_empty());
    let id = lines.next().unwrap_or_default().to_owned();
    let mut step = RawStep {
        id,
        needs: Vec::new(),
        command: None,
        prompt: None,
        gate: None,
        cwd: None,
        worktree: None,
    };
    let rest: Vec<&str> = lines.collect();
    if gate {
        step.gate = Some(rest.join("\n"));
        return Ok(step);
    }
    let mut rest = rest.into_iter().skip_while(|line| line.is_empty());
    step.command = Some(rest.next().unwrap_or_default().to_owned());
    let mut prompt = Vec::new();
    for line in rest {
        if let Some(cwd) = line.strip_prefix("cwd:") {
            step.cwd = Some(cwd.trim().to_owned());
        } else if let Some(name) = line.strip_prefix("worktree:") {
            step.worktree = Some(name.trim().to_owned());
        } else {
            prompt.push(line);
        }
    }
    let prompt = prompt.join("\n");
    step.prompt = Some(prompt.trim().to_owned()).filter(|prompt| !prompt.is_empty());
    Ok(step)
}

/// The element ID that one end of an arrow is bound to.
fn bound<'a>(arrow: &'a Value, end: &str) -> Option<&'a str> {
    text_of(arrow.get(end)?, "elementId")
}

/// Read a drawn flow. `name` is the flow's name (the file name).
pub fn parse_drawing(text: &str, name: &str) -> Result<Flow, FlowError> {
    let scene: Value =
        serde_json::from_str(text).map_err(|error| FlowError::Syntax(error.to_string()))?;
    if text_of(&scene, "type") != Some("excalidraw") {
        return Err(error("This file is not an Excalidraw drawing"));
    }
    let elements: Vec<&Value> = scene
        .get("elements")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|element| element.get("isDeleted").and_then(Value::as_bool) != Some(true))
        .collect();
    // The text inside each shape. `originalText` has no wrap line breaks.
    let mut texts = HashMap::new();
    for element in &elements {
        if text_of(element, "type") == Some("text")
            && let Some(container) = text_of(element, "containerId")
        {
            let text = text_of(element, "originalText").or(text_of(element, "text"));
            texts.insert(container, text.unwrap_or_default());
        }
    }
    let mut shapes = HashMap::new();
    for element in &elements {
        let kind = text_of(element, "type").unwrap_or_default();
        let (Some(id), true) = (
            text_of(element, "id"),
            matches!(kind, "rectangle" | "ellipse" | "diamond"),
        ) else {
            continue;
        };
        let Some(text) = texts.get(id).filter(|text| !text.trim().is_empty()) else {
            continue;
        };
        shapes.insert(
            id,
            Shape {
                place: place(element),
                step: step_of(text, kind == "diamond")?,
            },
        );
    }
    for arrow in elements
        .iter()
        .filter(|element| text_of(element, "type") == Some("arrow"))
    {
        let ends = (bound(arrow, "startBinding"), bound(arrow, "endBinding"));
        let (Some(from), Some(to)) = ends else {
            return Err(error(
                "An arrow is not connected to a step at both ends. Drag each end onto a box.",
            ));
        };
        let Some(needs) = shapes.get(from).map(|shape| shape.step.id.clone()) else {
            return Err(error("An arrow starts at a shape that has no text"));
        };
        let Some(shape) = shapes.get_mut(to) else {
            return Err(error("An arrow ends at a shape that has no text"));
        };
        if !shape.step.needs.contains(&needs) {
            shape.step.needs.push(needs);
        }
    }
    // Top to bottom, then left to right, so the order is stable.
    let mut shapes: Vec<Shape> = shapes.into_values().collect();
    shapes.sort_by(|a, b| {
        a.place
            .partial_cmp(&b.place)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.step.id.cmp(&b.step.id))
    });
    build(name, shapes.into_iter().map(|shape| shape.step).collect())
}

/// One Excalidraw element with the fields that every element has.
fn element(id: &str, kind: &str, rect: [f64; 4]) -> Value {
    let [x, y, width, height] = rect;
    json!({
        "id": id, "type": kind, "x": x, "y": y, "width": width, "height": height,
        "angle": 0, "strokeColor": "#1e1e1e", "backgroundColor": "transparent",
        "fillStyle": "solid", "strokeWidth": 2, "strokeStyle": "solid", "roughness": 1,
        "opacity": 100, "groupIds": [], "frameId": null, "roundness": {"type": 3},
        "seed": 1, "version": 1, "versionNonce": 1, "isDeleted": false,
        "boundElements": [], "updated": 1, "link": null, "locked": false,
    })
}

fn set(element: &mut Value, key: &str, value: Value) {
    if let Some(map) = element.as_object_mut() {
        map.insert(key.to_owned(), value);
    }
}

/// A text element; `container` puts it inside a shape.
fn text_element(id: &str, text: &str, rect: [f64; 4], container: Option<&str>) -> Value {
    let mut item = element(id, "text", rect);
    for (key, value) in [
        ("text", json!(text)),
        ("originalText", json!(text)),
        ("fontSize", json!(16)),
        ("fontFamily", json!(1)),
        ("lineHeight", json!(1.25)),
        ("autoResize", json!(true)),
        ("roundness", Value::Null),
        ("containerId", json!(container)),
        (
            "textAlign",
            json!(if container.is_some() {
                "center"
            } else {
                "left"
            }),
        ),
        (
            "verticalAlign",
            json!(if container.is_some() { "middle" } else { "top" }),
        ),
    ] {
        set(&mut item, key, value);
    }
    item
}

/// A shape with its text inside and the arrows bound to it.
fn shape(id: &str, kind: &str, rect: [f64; 4], (text, arrows): (&str, &[&str])) -> [Value; 2] {
    let text_id = format!("{id}-text");
    let mut item = element(id, kind, rect);
    let mut links = vec![json!({"type": "text", "id": text_id})];
    links.extend(
        arrows
            .iter()
            .map(|arrow| json!({"type": "arrow", "id": arrow})),
    );
    set(&mut item, "boundElements", json!(links));
    // The text block in the centre of the shape. A diamond has half its
    // width for text.
    let [x, y, width, height] = rect;
    let text_width = if kind == "diamond" {
        width * 0.5
    } else {
        width - 20.0
    };
    let text_height = f64::from(u32::try_from(text.lines().count()).unwrap_or(1)) * LINE;
    let inside = [
        width.mul_add(0.5, x) - text_width * 0.5,
        height.mul_add(0.5, y) - text_height * 0.5,
        text_width,
        text_height,
    ];
    [item, text_element(&text_id, text, inside, Some(id))]
}

/// Height of one text line: font size 16 with line height 1.25.
const LINE: f64 = 20.0;

/// An arrow from one shape to the next, on one horizontal line.
fn arrow(id: &str, (from, to): (&str, &str), (x, y, length): (f64, f64, f64)) -> Value {
    let mut item = element(id, "arrow", [x, y, 0.0, length]);
    let binding = |shape: &str| json!({"elementId": shape, "focus": 0, "gap": 6});
    for (key, value) in [
        ("points", json!([[0, 0], [0, length]])),
        ("startBinding", binding(from)),
        ("endBinding", binding(to)),
        ("startArrowhead", Value::Null),
        ("endArrowhead", json!("arrow")),
        ("roundness", json!({"type": 2})),
    ] {
        set(&mut item, key, value);
    }
    item
}

const LEGEND: &str = "How to draw a flow\n\
(Shift+1 shows the full drawing)\n\
\n\
A box is a step:\n\
- line 1: the step ID (a-z, 0-9, - or _)\n\
- line 2: the command\n\
- next lines: the prompt\n\
- optional lines: cwd: FOLDER, worktree: NAME\n\
\n\
A diamond is a gate:\n\
- line 1: the ID\n\
- next lines: the question for you\n\
\n\
An arrow from A to B: B starts after A is done.\n\
Drag each end of the arrow onto a shape.\n\
\n\
Codex: put the prompt in the command line,\n\
for example: codex 'Write the plan.'\n\
\n\
Other text and shapes are notes, like this one.";

/// The drawing of a new flow: plan, a gate, then build, with a legend.
pub fn template() -> String {
    let mut elements = Vec::new();
    elements.extend(shape(
        "plan",
        "rectangle",
        [20.0, -300.0, 280.0, 100.0],
        (
            "plan\nclaude\nWrite the plan in PLAN.md.",
            &["plan-approve"],
        ),
    ));
    elements.extend(shape(
        "approve",
        "diamond",
        [0.0, -140.0, 320.0, 170.0],
        (
            "approve\nIs the plan correct?",
            &["plan-approve", "approve-build"],
        ),
    ));
    elements.extend(shape(
        "build",
        "rectangle",
        [20.0, 90.0, 280.0, 100.0],
        ("build\nclaude\nDo the plan in PLAN.md.", &["approve-build"]),
    ));
    // Top to bottom, so the drawing fits a narrow pane.
    elements.push(arrow(
        "plan-approve",
        ("plan", "approve"),
        (160.0, -194.0, 48.0),
    ));
    elements.push(arrow(
        "approve-build",
        ("approve", "build"),
        (160.0, 36.0, 48.0),
    ));
    elements.push(text_element(
        "legend",
        LEGEND,
        [380.0, -300.0, 420.0, 420.0],
        None,
    ));
    json!({
        "type": "excalidraw", "version": 2, "source": "terminator",
        "elements": elements,
        "appState": {"viewBackgroundColor": "#ffffff", "gridSize": null},
        "files": {},
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::super::StepKind;
    use super::*;

    fn scene(elements: Vec<Value>) -> String {
        json!({"type": "excalidraw", "version": 2, "elements": elements}).to_string()
    }

    fn boxes(a: &str, b: &str) -> Vec<Value> {
        let mut elements = Vec::new();
        elements.extend(shape(
            "a",
            "rectangle",
            [0.0, 0.0, 200.0, 80.0],
            (a, &["ab"]),
        ));
        elements.extend(shape(
            "b",
            "rectangle",
            [300.0, 0.0, 200.0, 80.0],
            (b, &["ab"]),
        ));
        elements
    }

    #[test]
    fn new_flow_template_is_a_correct_flow() {
        let flow = parse_drawing(&template(), "feature").unwrap();
        assert_eq!(flow.name, "feature");
        let ids: Vec<_> = flow.steps.iter().map(|step| step.id.as_str()).collect();
        assert_eq!(ids, ["plan", "approve", "build"]);
        assert_eq!(
            flow.step("plan").unwrap().kind,
            StepKind::Command {
                command: "claude".into(),
                prompt: Some("Write the plan in PLAN.md.".into()),
                cwd: None,
                worktree: None,
            }
        );
        let gate = flow.step("approve").unwrap();
        assert_eq!(gate.needs, ["plan"]);
        assert_eq!(
            gate.kind,
            StepKind::Gate {
                message: "Is the plan correct?".into()
            }
        );
        assert_eq!(flow.step("build").unwrap().needs, ["approve"]);
    }

    #[test]
    fn arrow_gives_needs_and_option_lines_are_not_the_prompt() {
        let mut elements = boxes(
            "one\ncodex 'Do it.'",
            "two\nclaude\ncwd: crates/app\nworktree: two\nLine 1.\nLine 2.",
        );
        elements.push(arrow("ab", ("a", "b"), (206.0, 40.0, 88.0)));
        let flow = parse_drawing(&scene(elements), "n").unwrap();
        assert!(flow.step("one").unwrap().needs.is_empty());
        let two = flow.step("two").unwrap();
        assert_eq!(two.needs, ["one"]);
        assert_eq!(
            two.kind,
            StepKind::Command {
                command: "claude".into(),
                prompt: Some("Line 1.\nLine 2.".into()),
                cwd: Some("crates/app".into()),
                worktree: Some("two".into()),
            }
        );
    }

    #[test]
    fn arrow_with_a_free_end_is_refused() {
        let mut elements = boxes("one\nclaude", "two\nclaude");
        let mut free = arrow("ab", ("a", "b"), (206.0, 40.0, 88.0));
        set(&mut free, "endBinding", Value::Null);
        elements.push(free);
        let error = parse_drawing(&scene(elements), "n").unwrap_err();
        assert!(matches!(error, FlowError::Drawing(text) if text.contains("both ends")));
    }

    #[test]
    fn notes_deleted_shapes_and_wrap_line_breaks_are_ignored() {
        let mut elements = boxes("one\nclaude\nA long prompt.", "gone\nclaude");
        // Excalidraw wraps `text`; `originalText` is what the user typed.
        set(
            &mut elements[1],
            "text",
            json!("one\nclaude\nA long\nprompt."),
        );
        set(&mut elements[2], "isDeleted", json!(true));
        elements.push(element("empty", "rectangle", [0.0, 200.0, 50.0, 50.0]));
        elements.push(text_element(
            "note",
            "just a note",
            [0.0, 300.0, 90.0, 20.0],
            None,
        ));
        let flow = parse_drawing(&scene(elements), "n").unwrap();
        assert_eq!(flow.steps.len(), 1);
        assert_eq!(
            flow.steps[0].kind,
            StepKind::Command {
                command: "claude".into(),
                prompt: Some("A long prompt.".into()),
                cwd: None,
                worktree: None,
            }
        );
    }

    #[test]
    fn drawing_gets_the_same_checks_as_a_toml_flow() {
        let one = |text: &str, kind: &str| {
            let elements = shape("a", kind, [0.0, 0.0, 200.0, 80.0], (text, &[])).to_vec();
            parse_drawing(&scene(elements), "n")
        };
        assert_eq!(
            one("plan", "rectangle"),
            Err(FlowError::Text("plan".into(), "command"))
        );
        assert_eq!(
            one("ok", "diamond"),
            Err(FlowError::Text("ok".into(), "gate"))
        );
        assert_eq!(
            one("Plan 1\nclaude", "rectangle"),
            Err(FlowError::BadId("Plan 1".into()))
        );
        assert_eq!(
            one("a\nclaude\ncwd: ../x", "rectangle"),
            Err(FlowError::Cwd("a".into()))
        );
        assert_eq!(
            parse_drawing(&scene(Vec::new()), "n"),
            Err(FlowError::Empty)
        );
        assert!(matches!(
            parse_drawing("{\"type\":\"other\"}", "n"),
            Err(FlowError::Drawing(_))
        ));
        // Two arrows in a ring are a cycle.
        let mut elements = boxes("one\nclaude", "two\nclaude");
        elements.push(arrow("ab", ("a", "b"), (206.0, 40.0, 88.0)));
        elements.push(arrow("ba", ("b", "a"), (206.0, 60.0, 88.0)));
        assert!(matches!(
            parse_drawing(&scene(elements), "n"),
            Err(FlowError::Cycle(_))
        ));
    }
}
