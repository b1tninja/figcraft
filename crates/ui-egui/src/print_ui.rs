//! File ▸ Print: the frames of the page (or the selected ones) on paper, to a printer of this
//! computer or to a network printer typed by address.
//!
//! The dialog chooses *what* is printed: which frames, on which paper, the way up and how a frame is
//! scaled onto the sheet. The engine lays the frames out ([`figcraft_engine::cmd::file::print_plan`]: a
//! print-ready PDF, one sheet per frame) and the destination sends it:
//!
//! * **System printer**: craft-ui's queue controls over craft-print's spooler (Windows spooler, CUPS).
//!   Listing the queues and sending the job run on their own threads.
//! * **Network printer** (cfg `network_print`): craft-ui's `printing::network` destination over
//!   [`crate::network_glue`], WS-Print or IPP by address, with the capability states of the printer, the
//!   per-job confirmation and a tracked job whose status lines name the printer.
//!
//! Programmatic calls never print: `file.print` only opens this dialog (AGENTS.md), and a network job
//! starts only from the Print button, after the person ticked the confirmation box there.
//! The web build has no spooler and no sockets: it says so and leaves printing to the browser's PDF
//! viewer (export the frames to PDF).

use egui::Context;
use figcraft_pdf::{Fit, FitOrientation, FitScale};

use crate::FigApp;
use crate::i18n::tr;
#[cfg(not(target_arch = "wasm32"))]
use crate::i18n::trf;

/// The unprinted margin on every sheet, in points (a quarter inch: printers cannot print to the edge).
pub const MARGIN: f64 = 18.0;

/// A paper size the dialog offers: its short and long side in points and the media name printers list.
pub struct Paper {
    pub name: &'static str,
    pub short: f64,
    pub long: f64,
    pub media: &'static str,
}

/// The sizes of the dialog (`name` is also the key in the catalogs).
pub const PAPERS: &[Paper] = &[
    Paper { name: "US Letter", short: 612.0, long: 792.0, media: "na_letter_8.5x11in" },
    Paper { name: "US Legal", short: 612.0, long: 1008.0, media: "na_legal_8.5x14in" },
    Paper { name: "Tabloid", short: 792.0, long: 1224.0, media: "na_ledger_11x17in" },
    Paper { name: "A3", short: 841.89, long: 1190.55, media: "iso_a3_297x420mm" },
    Paper { name: "A4", short: 595.28, long: 841.89, media: "iso_a4_210x297mm" },
    Paper { name: "A5", short: 419.53, long: 595.28, media: "iso_a5_148x210mm" },
];

/// Index of A4 in [`PAPERS`], the opening choice.
const A4: usize = 4;

/// Which frames are printed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// Every visible frame of the page, in reading order.
    #[default]
    Page,
    /// The selected top-level frames.
    Selection,
}

/// Where the job goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Destination {
    #[default]
    System,
    Network,
}

/// The Print dialog's state.
pub struct PrintUi {
    pub open: bool,
    pub scope: Scope,
    /// An index into [`PAPERS`]; `None`: each sheet is its frame's own size (the printer fits it).
    pub paper: Option<usize>,
    pub orientation: FitOrientation,
    pub scale: FitScale,
    pub destination: Destination,
    /// The system printer queues and the last job sent to one.
    #[cfg(not(target_arch = "wasm32"))]
    pub spooler: craft_ui::printing::State,
    /// The network destination.
    #[cfg(network_print)]
    pub(crate) net: crate::network_glue::NetTasks,
}

impl Default for PrintUi {
    fn default() -> Self {
        Self {
            open: false,
            scope: Scope::Page,
            paper: Some(A4),
            orientation: FitOrientation::Auto,
            scale: FitScale::Shrink,
            destination: Destination::System,
            #[cfg(not(target_arch = "wasm32"))]
            spooler: craft_ui::printing::State::default(),
            #[cfg(network_print)]
            net: crate::network_glue::NetTasks::default(),
        }
    }
}

impl PrintUi {
    /// The layout the settings ask for: `None` when sheets are frame-sized.
    pub fn fit(&self) -> Option<Fit> {
        let paper = PAPERS.get(self.paper?)?;
        Some(Fit { paper: (paper.short, paper.long), orientation: self.orientation, scale: self.scale, margin: MARGIN })
    }

    /// The media name of the chosen paper, for printers that list it.
    pub fn media(&self) -> Option<&'static str> {
        self.paper.and_then(|i| PAPERS.get(i)).map(|p| p.media)
    }
}

/// The media name of a sheet size name of the dialog (the catalog key), for the sizes printers list.
pub fn media_for_paper(name: &str) -> Option<&'static str> {
    PAPERS.iter().find(|p| p.name == name).map(|p| p.media)
}

/// What the dialog would print: the frames and how many sheets they make.
#[cfg(not(target_arch = "wasm32"))]
fn frames(app: &FigApp, scope: Scope) -> Vec<figcraft_doc::NodeId> {
    let Ok(ds) = app.session.ds() else { return Vec::new() };
    use figcraft_engine::cmd::file::printable_frames;
    let selection = app.session.selection();
    match scope {
        Scope::Page => printable_frames(&ds.doc, ds.page, None),
        Scope::Selection => printable_frames(&ds.doc, ds.page, Some(&selection)),
    }
}

/// The print job for the dialog's settings, or why there is none.
#[cfg(not(target_arch = "wasm32"))]
fn plan(app: &FigApp) -> Result<(std::sync::Arc<figcraft_doc::Document>, figcraft_engine::cmd::file::PrintPlan), String> {
    use figcraft_engine::cmd::file::print_plan;
    let frames = frames(app, app.ui.print.scope);
    let ds = app.session.ds().map_err(|e| e.to_string())?;
    let plan = print_plan(&ds.doc, &frames, app.ui.print.fit()).map_err(|e| e.to_string())?;
    // The document the raster path renders from: its colours are sRGB, like the PDF's.
    let doc = std::sync::Arc::new(ds.doc.in_srgb().into_owned());
    Ok((doc, plan))
}

/// File ▸ Print was asked for (`dialog.print`): open the dialog, or, in the web build, say why not.
pub fn request(app: &mut FigApp) {
    #[cfg(target_arch = "wasm32")]
    app.toast(tr("Printing needs the desktop app: export the frames to PDF and print that."));
    #[cfg(not(target_arch = "wasm32"))]
    {
        let p = &mut app.ui.print;
        p.open = true;
        #[cfg(network_print)]
        p.net.state.dialog_opened();
    }
}

/// Once a frame: collect finished work, also while the dialog is closed, and tell the person how a
/// network print went when the dialog is not there to show it.
pub fn poll(app: &mut FigApp, ctx: &Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let open = app.ui.print.open;
        #[cfg(network_print)]
        if let Some(notice) = app.ui.print.net.poll(ctx, open) {
            app.toast(notice);
        }
        let _ = open;
        if !open {
            app.ui.print.spooler.poll(ctx);
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, ctx);
}

#[cfg(target_arch = "wasm32")]
pub fn show(_app: &mut FigApp, _ctx: &Context) {}

/// The dialog.
#[cfg(not(target_arch = "wasm32"))]
pub fn show(app: &mut FigApp, ctx: &Context) {
    if !app.ui.print.open {
        return;
    }
    let all = frames(app, Scope::Page).len();
    let selected = frames(app, Scope::Selection).len();
    // Nothing selected: only the page's frames can be chosen.
    if selected == 0 && app.ui.print.scope == Scope::Selection {
        app.ui.print.scope = Scope::Page;
    }
    let t = crate::theme::Tokens::get(ctx);
    let mut open = true;
    let mut print_now = false;
    crate::dialogs::window(tr("Print"), ctx).open(&mut open).show(ctx, |ui| {
        ui.set_width(380.0);
        let p = &mut app.ui.print;
        crate::widgets::label(ui, tr("What to print"), t.text_secondary);
        if crate::widgets::radio(ui, p.scope == Scope::Page, &trf("Every frame on this page ({n})", &[("n", &all)])) {
            p.scope = Scope::Page;
        }
        ui.add_enabled_ui(selected > 0, |ui| {
            if crate::widgets::radio(ui, p.scope == Scope::Selection, &trf("The selected frames ({n})", &[("n", &selected)])) {
                p.scope = Scope::Selection;
            }
        });
        ui.add_space(8.0);
        egui::Grid::new("print_layout").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label(tr("Paper:"));
            let paper_text = p.paper.and_then(|i| PAPERS.get(i)).map_or_else(|| tr("Frame size").to_owned(), |paper| tr(paper.name).to_owned());
            egui::ComboBox::from_id_salt("print_paper").selected_text(paper_text).show_ui(ui, |ui| {
                ui.selectable_value(&mut p.paper, None, tr("Frame size"));
                for (i, paper) in PAPERS.iter().enumerate() {
                    ui.selectable_value(&mut p.paper, Some(i), tr(paper.name));
                }
            });
            ui.end_row();
            // Orientation and scale only mean something on a paper.
            ui.add_enabled_ui(p.paper.is_some(), |ui| {
                ui.label(tr("Orientation:"));
            });
            let orientation = match p.orientation {
                FitOrientation::Auto => tr("Automatic (by the frame's shape)"),
                FitOrientation::Portrait => tr("Portrait"),
                FitOrientation::Landscape => tr("Landscape"),
            };
            ui.add_enabled_ui(p.paper.is_some(), |ui| {
                egui::ComboBox::from_id_salt("print_orientation").selected_text(orientation).show_ui(ui, |ui| {
                    ui.selectable_value(&mut p.orientation, FitOrientation::Auto, tr("Automatic (by the frame's shape)"));
                    ui.selectable_value(&mut p.orientation, FitOrientation::Portrait, tr("Portrait"));
                    ui.selectable_value(&mut p.orientation, FitOrientation::Landscape, tr("Landscape"));
                });
            });
            ui.end_row();
            ui.add_enabled_ui(p.paper.is_some(), |ui| {
                ui.label(tr("Scale:"));
            });
            let scale = match p.scale {
                FitScale::Shrink => tr("Actual size, shrunk to fit when larger"),
                FitScale::Fill => tr("Fit to the page"),
            };
            ui.add_enabled_ui(p.paper.is_some(), |ui| {
                egui::ComboBox::from_id_salt("print_scale").selected_text(scale).show_ui(ui, |ui| {
                    ui.selectable_value(&mut p.scale, FitScale::Shrink, tr("Actual size, shrunk to fit when larger"));
                    ui.selectable_value(&mut p.scale, FitScale::Fill, tr("Fit to the page"));
                });
            });
            ui.end_row();
        });
        if p.paper.is_none() {
            crate::widgets::label(ui, tr("Each sheet is the size of its frame: the printer decides how it fits."), t.text_secondary);
        }
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);
        crate::widgets::label(ui, tr("Print to"), t.text_secondary);
        ui.horizontal(|ui| {
            if crate::widgets::radio(ui, p.destination == Destination::System, tr("A printer on this computer")) {
                p.destination = Destination::System;
            }
            #[cfg(network_print)]
            if crate::widgets::radio(ui, p.destination == Destination::Network, tr("A network printer")) {
                p.destination = Destination::Network;
            }
        });
        ui.add_space(6.0);
        let sheets_ok = match p.scope {
            Scope::Page => all > 0,
            Scope::Selection => selected > 0,
        };
        match p.destination {
            Destination::System => print_now = system_section(ui, p, &t, sheets_ok),
            #[cfg(network_print)]
            Destination::Network => {
                let response = p.net.show(ui, sheets_ok);
                ui.add_space(6.0);
                if crate::widgets::button(ui, tr("Print"), true, response.block.is_none()).clicked() {
                    print_now = true;
                }
            }
            #[cfg(not(network_print))]
            Destination::Network => p.destination = Destination::System,
        }
    });
    if print_now {
        send(app, ctx);
    }
    if !open {
        app.ui.print.open = false;
    }
}

/// The system printer's controls and its Print button. Returns whether Print was clicked.
#[cfg(not(target_arch = "wasm32"))]
fn system_section(ui: &mut egui::Ui, p: &mut PrintUi, t: &crate::theme::Tokens, sheets_ok: bool) -> bool {
    use craft_ui::printing::Feedback;
    let discover = spooler_discover();
    let labels = craft_ui::printing::Labels {
        printer: tr("Printer:"),
        select_printer: tr("Select a printer"),
        refresh: tr("Refresh"),
        copies: tr("Copies:"),
        printer_default: tr("Printer default"),
        duplex: tr("Two-sided:"),
        one_sided: tr("Off"),
        long_edge: tr("Flip on long edge"),
        short_edge: tr("Flip on short edge"),
        grayscale: tr("Print in black and white"),
    };
    p.spooler.show(ui, egui::Id::new("system-printer"), &discover, &labels);
    let message = match &p.spooler.message {
        None => None,
        Some(Feedback::FindingPrinters) => Some(tr("Looking for printers…").to_owned()),
        Some(Feedback::NoPrinters) => Some(tr("No printers were found on this computer.").to_owned()),
        Some(Feedback::DiscoveryFailed) => Some(tr("The printers couldn't be listed.").to_owned()),
        Some(Feedback::Submitting) => Some(tr("Sending to the printer…").to_owned()),
        Some(Feedback::Submitted(receipt)) => Some(trf("Sent to the printer: {receipt}", &[("receipt", receipt)])),
        Some(Feedback::Failed(error)) => Some(trf("The print job failed: {error}", &[("error", error)])),
        Some(Feedback::WorkerStopped) => Some(tr("The print job stopped unexpectedly.").to_owned()),
    };
    if let Some(message) = message {
        crate::widgets::label(ui, &message, t.text_secondary);
    }
    ui.add_space(6.0);
    crate::widgets::button(ui, tr("Print"), true, sheets_ok && p.spooler.ready()).clicked()
}

/// Start the job for the dialog's settings.
#[cfg(not(target_arch = "wasm32"))]
fn send(app: &mut FigApp, ctx: &Context) {
    let (doc, plan) = match plan(app) {
        Ok(planned) => planned,
        Err(why) => {
            app.toast(trf("Nothing was printed: {why}", &[("why", &why)]));
            return;
        }
    };
    let name = doc.name.clone();
    match app.ui.print.destination {
        Destination::System => {
            let submit = spooler_submit(name);
            let settings = app.ui.print.spooler.settings.clone();
            if app.ui.print.spooler.begin(submit(plan.pdf, settings)).is_err() {
                app.toast(tr("A print job is already being sent."));
            }
        }
        #[cfg(network_print)]
        Destination::Network => {
            let media = app.ui.print.media();
            if !app.ui.print.net.send(Some(ctx.clone()), &name, doc, plan, media) {
                app.toast(tr("Query the printer, then tick the confirmation box before sending."));
            }
        }
        #[cfg(not(network_print))]
        Destination::Network => {}
    }
    let _ = ctx;
}

/// Lists the system's printer queues on a thread of its own.
#[cfg(not(target_arch = "wasm32"))]
fn spooler_discover() -> craft_ui::printing::Discover {
    Box::new(|| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new().name("print-discovery".into()).spawn(move || {
            let printers = craft_print::printers().into_iter().map(|p| craft_ui::printing::Printer { name: p.name, default: p.default }).collect();
            let _ = sender.send(printers);
        });
        receiver
    })
}

/// Sends a print-ready PDF to a system queue on a thread of its own. Workers are kept until
/// [`finish_pending_jobs`]: dropping the handle at exit could kill the spooler hand-over half way.
#[cfg(not(target_arch = "wasm32"))]
fn spooler_submit(title: String) -> craft_ui::printing::Submit {
    Box::new(move |pdf, settings| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let title = title.clone();
        let worker = std::thread::Builder::new().name("print-submit".into()).spawn(move || {
            let job = craft_print::Job {
                printer: Some(settings.printer),
                copies: settings.copies.clamp(1, 999),
                duplex: settings.duplex.map(|d| match d {
                    craft_ui::printing::Duplex::Off => craft_print::Duplex::Off,
                    craft_ui::printing::Duplex::LongEdge => craft_print::Duplex::LongEdge,
                    craft_ui::printing::Duplex::ShortEdge => craft_print::Duplex::ShortEdge,
                }),
                grayscale: settings.grayscale,
                // The sheets are laid out on the paper already.
                scaling: craft_print::Scaling::Exact,
                title,
                ..craft_print::Job::default()
            };
            let _ = sender.send(craft_print::submit(&pdf, &job).map_err(|e| e.to_string()));
        });
        if let Ok(worker) = worker {
            let mut jobs = JOBS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            jobs.retain(|j| !j.is_finished());
            jobs.push(worker);
        }
        receiver
    })
}

#[cfg(not(target_arch = "wasm32"))]
static JOBS: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>> = std::sync::Mutex::new(Vec::new());

/// Wait for the print jobs being handed to the spooler (call before the process exits).
#[cfg(not(target_arch = "wasm32"))]
pub fn finish_pending_jobs() {
    let workers = std::mem::take(&mut *JOBS.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    for worker in workers {
        if worker.join().is_err() {
            log::error!("the print worker stopped unexpectedly during shutdown");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dialog_opens_on_a4_and_maps_its_choices_to_the_layout() {
        let mut ui = PrintUi::default();
        assert_eq!(ui.media(), Some("iso_a4_210x297mm"));
        let fit = ui.fit().expect("a paper");
        assert_eq!((fit.paper, fit.margin, fit.scale, fit.orientation), ((595.28, 841.89), MARGIN, FitScale::Shrink, FitOrientation::Auto));
        ui.paper = Some(0);
        ui.scale = FitScale::Fill;
        ui.orientation = FitOrientation::Landscape;
        assert_eq!(ui.media(), Some("na_letter_8.5x11in"));
        assert_eq!(ui.fit().map(|f| (f.paper, f.scale, f.orientation)), Some(((612.0, 792.0), FitScale::Fill, FitOrientation::Landscape)));
        // Frame size: no paper, no layout, no media.
        ui.paper = None;
        assert!(ui.fit().is_none() && ui.media().is_none());
        ui.paper = Some(99);
        assert!(ui.fit().is_none() && ui.media().is_none(), "an index outside the table is no paper");
    }

    #[test]
    fn paper_names_map_to_the_sizes_printers_list() {
        assert_eq!(media_for_paper("A4"), Some("iso_a4_210x297mm"));
        assert_eq!(media_for_paper("US Letter"), Some("na_letter_8.5x11in"));
        assert_eq!(media_for_paper("ARCH A (9 x 12 in)"), None);
    }

    #[test]
    fn every_paper_is_portrait_and_names_a_media() {
        for p in PAPERS {
            assert!(p.short < p.long, "{}", p.name);
            assert!(!p.media.is_empty());
        }
    }
}
