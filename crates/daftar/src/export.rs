//! Export as PDF (Ctrl+Shift+E), as the Mac's: the reader's page printed to
//! an A4 PDF where the person chooses. The page's print styles (shared with
//! the Mac in `web/src/reader.css`) give the margins, the Daylight colors,
//! every section open and none of the page's controls.

use adw::prelude::*;
use gtk::gio;

pub fn pdf(window: &gtk::Window, view: &webkit6::WebView, title: &str, notice: impl Fn(&str) + Clone + 'static) {
    let dialog = gtk::FileDialog::builder().title("Export as PDF").modal(true).initial_name(format!("{title}.pdf")).build();
    let view = view.clone();
    dialog.save(Some(window), gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else { return };
        let settings = gtk::PrintSettings::new();
        settings.set_printer("Print to File");
        settings.set(gtk::PRINT_SETTINGS_OUTPUT_FILE_FORMAT, Some("pdf"));
        settings.set(gtk::PRINT_SETTINGS_OUTPUT_URI, Some(&file.uri()));
        let setup = gtk::PageSetup::new();
        setup.set_paper_size(&gtk::PaperSize::new(Some(gtk::PAPER_NAME_A4)));
        // The page's `@page` rule has the margins, as on the Mac.
        for set in [
            gtk::PageSetup::set_top_margin,
            gtk::PageSetup::set_bottom_margin,
            gtk::PageSetup::set_left_margin,
            gtk::PageSetup::set_right_margin,
        ] {
            set(&setup, 0.0, gtk::Unit::Mm);
        }
        let operation = webkit6::PrintOperation::new(&view);
        operation.set_print_settings(&settings);
        operation.set_page_setup(&setup);
        let name = file.basename().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let (done, failed) = (notice.clone(), notice);
        operation.connect_finished(move |_| done(&format!("Saved {name}.")));
        operation.connect_failed(move |_, error| failed(&format!("The PDF was not saved: {error}")));
        operation.print();
    });
}
