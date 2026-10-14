//! Slots on WinUI: a placeholder where a layout placed one, and a board of
//! what the pages and layouts below filled it with.
//!
//! The layout draws before its child, so it cannot wait for the fill. It
//! places a small component of its own instead, and a fill reaches that
//! component as a message - a drain later, redrawing it and nothing around
//! it.

use std::any::TypeId;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use guinea_core::scope::Scope;
use guinea_core::trace::Cause;
use guinea_router::slot::Slot;
use windows_reactor::{Border, Component, ComponentContext, LocalSender, View, ViewContext};

/// A slot as one layout placed it: the placing layout's scope, and which
/// slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Place {
    placer: Scope,
    slot: TypeId,
}

/// One node's fill of one place.
struct Fill {
    filler: Scope,
    depth: usize,
    view: View,
    made: bool,
}

#[derive(Default)]
struct Entry {
    fills: Vec<Fill>,
    /// The placeholder drawing this place, and which one it is: a layout
    /// that moves the slot draws a new one before the old one goes.
    shown: Option<(u64, LocalSender<Option<Cause>>)>,
}

impl Entry {
    fn winner(&self) -> Option<&Fill> {
        self.fills.iter().max_by_key(|fill| fill.depth)
    }

    fn tell(&self) {
        if let Some((_, shown)) = &self.shown {
            let _gone = !shown.send(guinea_core::trace::current());
        }
    }
}

#[derive(Default)]
struct Board {
    entries: HashMap<Place, Entry>,
    placed: HashMap<Scope, HashSet<TypeId>>,
}

thread_local! {
    static BOARD: RefCell<Board> = RefCell::new(Board::default());
}

/// Before a node draws: what it placed and filled last time is forgotten
/// unless it says so again.
pub(crate) fn drawing(key: Scope) {
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

/// After a node draws: a fill it did not make again is withdrawn.
pub(crate) fn drawn(key: Scope) {
    withdraw(key, |fill| !fill.made);
}

/// When a node leaves the screen: everything it filled goes with it.
pub(crate) fn left(filler: Scope) {
    withdraw(filler, |_| true);

    BOARD.with(|board| {
        board.borrow_mut().placed.remove(&filler);
    });
}

fn withdraw(filler: Scope, going: impl Fn(&Fill) -> bool) {
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

/// The placeholder for `S`, where the layout in `placer` draws it, showing
/// `standing` while nothing fills it.
pub(crate) fn place<S: Slot>(placer: Scope, standing: Option<View>) -> View {
    let slot = TypeId::of::<S>();

    BOARD.with(|board| {
        board
            .borrow_mut()
            .placed
            .entry(placer)
            .or_default()
            .insert(slot);
    });

    View::component::<Placeholder>(Shown {
        place: Place { placer, slot },
        name: guinea_router::observability::short(std::any::type_name::<S>()),
        standing,
    })
}

/// Hands `view` to the nearest layout above `filler` that placed `S`.
pub(crate) fn fill<S: Slot>(filler: Scope, view: View) {
    let slot = TypeId::of::<S>();
    let above: Vec<Scope> = std::iter::successors(filler.parent(), Scope::parent).collect();

    let placer = BOARD.with(|board| {
        let board = board.borrow();
        above.iter().copied().find(|placer| {
            board
                .placed
                .get(placer)
                .is_some_and(|placed| placed.contains(&slot))
        })
    });

    let Some(placer) = placer else {
        unplaced::<S>(filler);
        return;
    };
    let depth = above.len();

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
                    depth,
                    view,
                    made: true,
                });
                entry.tell();
            }
        }
    });
}

fn unplaced<S: Slot>(filler: Scope) {
    let short = guinea_router::observability::short;
    let names: Vec<&str> = std::iter::successors(Some(filler), Scope::parent)
        .filter_map(guinea_router::mount::name)
        .map(short)
        .collect();
    let said = format!(
        "`{}` fills `{}`, but no layout above it places that slot:\n  {}",
        names.first().copied().unwrap_or("this view"),
        short(std::any::type_name::<S>()),
        names.join(" <- "),
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
    /// What the layout shows here while nothing fills it.
    standing: Option<View>,
}

/// Where a slot is drawn: the winning fill, what stands in for one, or
/// nothing.
pub(crate) struct Placeholder {
    id: u64,
}

impl Component for Placeholder {
    type Input = Shown;
    type Message = Option<Cause>;

    fn create(_input: &Shown, _cx: &ComponentContext<Self>) -> Self {
        thread_local! {
            static NEXT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
        }

        let id = NEXT.get();
        NEXT.set(id + 1);
        Self { id }
    }

    fn update(&mut self, cause: Option<Cause>, _cx: &ComponentContext<Self>) {
        crate::winui::handling(cause);
    }

    fn view(&self, input: &Shown, cx: &mut ViewContext<Self>) -> View {
        let _caused = crate::winui::caused(cx);
        let place = input.place;
        let id = self.id;
        let sender = cx.sender();

        let shown = BOARD.with(|board| {
            let mut board = board.borrow_mut();
            let entry = board.entries.entry(place).or_default();
            entry.shown = Some((id, sender));
            entry.winner().map(|fill| fill.view.clone())
        });

        cx.use_effect("guinea.slot", place, move || {
            Some(Box::new(move || {
                BOARD.with(|board| {
                    let mut board = board.borrow_mut();
                    if let Some(entry) = board.entries.get_mut(&place)
                        && entry.shown.as_ref().is_some_and(|(shown, _)| *shown == id)
                    {
                        entry.shown = None;
                    }
                    board.entries.retain(|_, entry| {
                        !entry.fills.is_empty() || entry.shown.is_some()
                    });
                });
            }) as Box<dyn FnOnce()>)
        });

        let border = Border::new().automation_id(input.name);
        match shown.or_else(|| input.standing.clone()) {
            Some(view) => border.content(view).into(),
            None => border.into(),
        }
    }
}
