//! Slots on WinUI: a placeholder where a layout placed one, and a board of
//! what the segments below filled it with.
//!
//! The layout draws before its child, so it cannot wait for the fill. It
//! places a small component of its own instead, and a fill reaches that
//! component as a message - a drain later, redrawing it and nothing around
//! it.

use std::any::TypeId;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use guinea_router::router::SegmentProps;
use guinea_router::slot::Slot;
use windows_reactor::{Border, Component, ComponentContext, LocalSender, View, ViewContext};

use crate::WinUi;

/// A slot as one layout placed it: the placing segment's scope, and which
/// slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Place {
    placer: usize,
    slot: TypeId,
}

/// One segment's fill of one place.
struct Fill {
    filler: usize,
    depth: usize,
    view: View,
    made: bool,
}

#[derive(Default)]
struct Entry {
    fills: Vec<Fill>,
    shown: Option<LocalSender<()>>,
}

impl Entry {
    fn winner(&self) -> Option<&Fill> {
        self.fills.iter().max_by_key(|fill| fill.depth)
    }

    fn tell(&self) {
        if let Some(shown) = &self.shown {
            let _gone = !shown.send(());
        }
    }
}

#[derive(Default)]
struct Board {
    entries: HashMap<Place, Entry>,
    placed: HashMap<usize, HashSet<TypeId>>,
}

thread_local! {
    static BOARD: RefCell<Board> = RefCell::new(Board::default());
}

/// Before a segment draws: what it placed and filled last time is forgotten
/// unless it says so again.
pub(crate) fn drawing(props: &SegmentProps<WinUi>) {
    let key = props.scopes[props.cursor].key();

    BOARD.with(|board| {
        let mut board = board.borrow_mut();
        board.placed.remove(&key);
        for entry in board.entries.values_mut() {
            for fill in entry.fills.iter_mut().filter(|fill| fill.filler == key) {
                fill.made = false;
            }
        }
    });
}

/// After a segment draws: a fill it did not make again is withdrawn.
pub(crate) fn drawn(props: &SegmentProps<WinUi>) {
    let key = props.scopes[props.cursor].key();
    withdraw(key, |fill| !fill.made);
}

/// When a segment leaves the screen: everything it filled goes with it.
pub(crate) fn left(filler: usize) {
    withdraw(filler, |_| true);

    BOARD.with(|board| {
        board.borrow_mut().placed.remove(&filler);
    });
}

fn withdraw(filler: usize, going: impl Fn(&Fill) -> bool) {
    BOARD.with(|board| {
        let mut board = board.borrow_mut();
        for entry in board.entries.values_mut() {
            let before = entry.fills.len();
            entry
                .fills
                .retain(|fill| fill.filler != filler || !going(fill));
            if entry.fills.len() != before {
                entry.tell();
            }
        }
        board
            .entries
            .retain(|_, entry| !entry.fills.is_empty() || entry.shown.is_some());
    });
}

/// The placeholder for `S`, where the layout at `props` draws it.
pub(crate) fn place<S: Slot>(props: &SegmentProps<WinUi>) -> View {
    let placer = props.scopes[props.cursor].key();
    let slot = TypeId::of::<S>();

    BOARD.with(|board| {
        board
            .borrow_mut()
            .placed
            .entry(placer)
            .or_default()
            .insert(slot);
    });

    let part = props
        .part::<S>()
        .map(|part| part.chain[part.cursor].mount.view(part, &()));

    View::component::<Placeholder>(Shown {
        place: Place { placer, slot },
        name: guinea_router::observability::short(std::any::type_name::<S>()),
        part,
    })
}

/// Hands `view` to the nearest layout above `props` that placed `S`.
pub(crate) fn fill<S: Slot>(props: &SegmentProps<WinUi>, view: View) {
    let slot = TypeId::of::<S>();
    let filler = props.scopes[props.cursor].key();

    let placer = BOARD.with(|board| {
        let board = board.borrow();
        (0..props.cursor).rev().find_map(|at| {
            let placer = props.scopes[at].key();
            board
                .placed
                .get(&placer)
                .is_some_and(|placed| placed.contains(&slot))
                .then_some(placer)
        })
    });

    let Some(placer) = placer else {
        unplaced::<S>(props);
        return;
    };

    BOARD.with(|board| {
        let mut board = board.borrow_mut();
        let entry = board.entries.entry(Place { placer, slot }).or_default();

        match entry.fills.iter_mut().find(|fill| fill.filler == filler) {
            Some(fill) => {
                fill.made = true;
                if fill.view != view {
                    fill.view = view;
                    entry.tell();
                }
            }
            None => {
                entry.fills.push(Fill {
                    filler,
                    depth: props.cursor,
                    view,
                    made: true,
                });
                entry.tell();
            }
        }
    });
}

fn unplaced<S: Slot>(props: &SegmentProps<WinUi>) {
    let name = |at: usize| guinea_router::observability::short((props.chain[at].type_name)());
    let walked = (0..=props.cursor)
        .rev()
        .map(name)
        .collect::<Vec<_>>()
        .join(" <- ");
    let said = format!(
        "`{}` fills `{}`, but no layout above it places that slot:\n  {walked}",
        name(props.cursor),
        guinea_router::observability::short(std::any::type_name::<S>()),
    );

    if cfg!(debug_assertions) {
        panic!("{said}");
    }
    tracing::warn!("{said}");
}

#[derive(Clone, PartialEq)]
pub(crate) struct Shown {
    place: Place,
    name: &'static str,
    /// What `routes!` mounted in this slot, shown when nothing fills it.
    part: Option<View>,
}

/// Where a slot is drawn: the winning fill, the part, or nothing.
pub(crate) struct Placeholder;

impl Component for Placeholder {
    type Input = Shown;
    type Message = ();

    fn create(_input: &Shown, _cx: &ComponentContext<Self>) -> Self {
        Self
    }

    fn view(&self, input: &Shown, cx: &mut ViewContext<Self>) -> View {
        let place = input.place;
        let sender = cx.sender();

        let shown = BOARD.with(|board| {
            let mut board = board.borrow_mut();
            let entry = board.entries.entry(place).or_default();
            entry.shown = Some(sender);
            entry.winner().map(|fill| fill.view.clone())
        });

        cx.use_effect("guinea.slot", place, move || {
            Some(Box::new(move || {
                BOARD.with(|board| {
                    let mut board = board.borrow_mut();
                    if let Some(entry) = board.entries.get_mut(&place) {
                        entry.shown = None;
                    }
                    board.entries.retain(|_, entry| {
                        !entry.fills.is_empty() || entry.shown.is_some()
                    });
                });
            }) as Box<dyn FnOnce()>)
        });

        let border = Border::new().automation_id(input.name);
        match shown.or_else(|| input.part.clone()) {
            Some(view) => border.content(view).into(),
            None => border.into(),
        }
    }
}
