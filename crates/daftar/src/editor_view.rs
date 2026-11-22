//! The editor's text view: a GtkSourceView that lets the editor draw under
//! and over the text, in the buffer's coordinates. GtkTextView draws only
//! text and the backgrounds of its tags, and its own caret in the text's
//! colour, so what the editor draws itself is drawn here.

use gtk::glib;
use gtk::subclass::prelude::*;

/// Draws one layer of the view: under the text, or over it.
type Painter = Box<dyn Fn(&gtk::TextView, gtk::TextViewLayer, &gtk::Snapshot)>;

mod imp {
    use std::cell::RefCell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use sourceview5::subclass::prelude::*;

    use super::Painter;

    #[derive(Default)]
    pub struct EditorView {
        pub painter: RefCell<Option<Painter>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for EditorView {
        const NAME: &'static str = "DaftarEditorView";
        type Type = super::EditorView;
        type ParentType = sourceview5::View;
    }

    impl ObjectImpl for EditorView {}
    impl WidgetImpl for EditorView {}

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

    /// [painter] draws each layer after the view's own.
    pub fn set_painter(&self, painter: impl Fn(&gtk::TextView, gtk::TextViewLayer, &gtk::Snapshot) + 'static) {
        *self.imp().painter.borrow_mut() = Some(Box::new(painter));
    }
}
