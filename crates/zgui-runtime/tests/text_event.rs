//! The `text` event: the text a key press is about to insert, announced before it is inserted.
//!
//! A listener on a field or on any of its ancestors hears the text, and a listener that prevents
//! the event keeps the text out of the field. Every event here is a platform event handed to a real
//! window, as in `value.rs`.

mod support;

use std::cell::RefCell;
use std::rc::Rc;

use zgui_platform::SurfaceEvent;
use zgui_view::{BuildCx, EventCx, IntoView, NodeRef, View, events};
use zgui_vocab::{Key, KeyCode, KeyEvent, KeyState, Modifiers, NamedKey, PhysicalKey, Timestamp};

/// The sheet the fixture is styled by.
const CSS: &str = "root { display: block; width: 400px; height: 300px }
                   field { display: block; width: 200px; height: 40px }";

/// A press of a character key.
fn letter(text: &str) -> KeyEvent {
    KeyEvent {
        key: Key::Character(text.into()),
        key_without_modifiers: Key::Character(text.into()),
        physical: PhysicalKey::Code(KeyCode::KeyA),
        location: zgui_vocab::KeyLocation::Standard,
        repeat: false,
    }
}

/// A window holding one focused field. `refuse` is the text the field's `text` listener prevents.
struct Script {
    harness: zgui_platform_headless::Harness<zgui_runtime::Runtime>,
    /// Every `text` event a listener saw, as `(where, text)`.
    seen: Rc<RefCell<Vec<(&'static str, String)>>>,
    /// Every value the field announced.
    values: Rc<RefCell<Vec<String>>>,
}

impl Script {
    fn press(&mut self, event: KeyEvent, modifiers: Modifiers) {
        self.harness.deliver_to_first(SurfaceEvent::Key {
            state: KeyState::Pressed,
            event,
            modifiers,
            timestamp: Timestamp::ORIGIN,
        });
        self.harness.settle(8);
    }
}

fn scripted(refuse: &'static str) -> Script {
    let field = NodeRef::new();
    let seen: Rc<RefCell<Vec<(&'static str, String)>>> = Rc::new(RefCell::new(Vec::new()));
    let values: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

    fn record(
        seen: &Rc<RefCell<Vec<(&'static str, String)>>>,
        at: &'static str,
        refuse: &'static str,
    ) -> impl Fn(&mut EventCx<'_, events::Text>) + use<> {
        let seen = Rc::clone(seen);
        move |cx: &mut EventCx<'_, events::Text>| {
            let text = cx
                .payload()
                .as_text()
                .expect("a text payload")
                .text
                .to_string();
            if at == "field" && text == refuse {
                cx.prevent_default();
            }
            seen.borrow_mut().push((at, text));
        }
    }

    let recorded = Rc::clone(&seen);
    let announced = Rc::clone(&values);
    let mut harness = support::app_with_text(CSS, move |cx: &mut BuildCx<'_>| {
        let announced = Rc::clone(&announced);
        Box::new(
            zgui_elements::column()
                .class("root")
                .on(events::TEXT, record(&recorded, "root", refuse))
                .child(
                    zgui_elements::field()
                        .node_ref(field)
                        .on(events::TEXT, record(&recorded, "field", refuse))
                        .on(events::INPUT, move |cx: &mut EventCx<'_, events::Input>| {
                            let value = cx.payload().as_value().expect("a value").value.to_string();
                            announced.borrow_mut().push(value);
                        }),
                )
                .into_view()
                .build(cx),
        )
    });
    harness.settle(8);
    harness.deliver_to_first(SurfaceEvent::Key {
        state: KeyState::Pressed,
        event: KeyEvent::named(NamedKey::Tab, PhysicalKey::Code(KeyCode::Tab)),
        modifiers: Modifiers::NONE,
        timestamp: Timestamp::ORIGIN,
    });
    harness.settle(8);
    Script {
        harness,
        seen,
        values,
    }
}

#[test]
fn a_key_that_inserts_text_announces_it_on_the_field_and_its_ancestors() {
    let mut script = scripted("");
    script.press(letter("q"), Modifiers::NONE);
    assert_eq!(
        *script.seen.borrow(),
        vec![("field", "q".to_owned()), ("root", "q".to_owned())],
        "the runtime dispatched no text event for a typed character"
    );
    assert_eq!(*script.values.borrow(), vec!["q".to_owned()]);
}

#[test]
fn preventing_the_text_event_keeps_the_text_out_of_the_field() {
    let mut script = scripted("x");
    script.press(letter("x"), Modifiers::NONE);
    assert!(
        script.values.borrow().is_empty(),
        "a prevented text event still typed into the field"
    );
    script.press(letter("y"), Modifiers::NONE);
    assert_eq!(*script.values.borrow(), vec!["y".to_owned()]);
}

#[test]
fn a_command_chord_and_a_key_that_inserts_nothing_announce_no_text() {
    let mut script = scripted("");
    script.press(letter("c"), Modifiers::CONTROL);
    script.press(
        KeyEvent::named(NamedKey::ArrowLeft, PhysicalKey::Code(KeyCode::ArrowLeft)),
        Modifiers::NONE,
    );
    assert!(
        script.seen.borrow().is_empty(),
        "{:?}",
        script.seen.borrow()
    );
}
