//! The editor's text view: a GtkSourceView that lets the editor draw under
//! and over the text, in the buffer's coordinates. GtkTextView draws only
//! text and the backgrounds of its tags, so a callout's panel, a chip's pill
//! and a task's box are drawn here (`editor_draw.rs`): they are a way to see
//! the text, never characters of the buffer.

use gtk::glib;
use gtk::subclass::prelude::*;

/// Draws one layer of the view: under the text, or over it.
type Painter = Box<dyn Fn(&gtk::TextView, gtk::TextViewLayer, &gtk::Snapshot)>;

mod imp {
    use std::cell::{Cell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use sourceview5::subclass::prelude::*;

    use super::Painter;

    #[derive(Default)]
    pub struct EditorView {
        pub painter: RefCell<Option<Painter>>,
        /// The height the view asks for, when its holder said one.
        pub natural_height: Cell<Option<i32>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for EditorView {
        const NAME: &'static str = "DaftarEditorView";
        type Type = super::EditorView;
        type ParentType = sourceview5::View;
    }

    impl ObjectImpl for EditorView {}
    impl WidgetImpl for EditorView {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let (minimum, natural, minimum_baseline, natural_baseline) = self.parent_measure(orientation, for_size);
            match self.natural_height.get().filter(|_| orientation == gtk::Orientation::Vertical) {
                // Only the height it would like: the least it takes stays, so
                // a holder already on screen is never smaller than its least.
                Some(height) => (minimum, height.max(minimum), minimum_baseline, natural_baseline),
                None => (minimum, natural, minimum_baseline, natural_baseline),
            }
        }
    }

    impl TextViewImpl for EditorView {
        fn snapshot_layer(&self, layer: gtk::TextViewLayer, snapshot: gtk::Snapshot) {
            self.parent_snapshot_layer(layer, snapshot.clone());
            if let Some(painter) = self.painter.borrow().as_ref() {
                painter(self.obj().upcast_ref(), layer, &snapshot);
            }
        }
    }

    impl ViewImpl for EditorView {}
}

glib::wrapper! {
    pub struct EditorView(ObjectSubclass<imp::EditorView>)
        @extends sourceview5::View, gtk::TextView, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Scrollable;
}

impl EditorView {
    pub fn new(buffer: &sourceview5::Buffer) -> EditorView {
        glib::Object::builder().property("buffer", buffer).build()
    }

    /// The height the view would like, for a holder that sizes itself by
    /// it (the link preview's card): a text view knows its lines' height
    /// only once it has laid them out. `None` is the view's own answer.
    pub fn set_natural_height(&self, height: Option<i32>) {
        if self.imp().natural_height.replace(height) != height {
            gtk::prelude::WidgetExt::queue_resize(self);
        }
    }

    pub fn natural_height(&self) -> Option<i32> {
        self.imp().natural_height.get()
    }

    /// [painter] draws each layer after the view's own.
    pub fn set_painter(&self, painter: impl Fn(&gtk::TextView, gtk::TextViewLayer, &gtk::Snapshot) + 'static) {
        *self.imp().painter.borrow_mut() = Some(Box::new(painter));
    }
}
