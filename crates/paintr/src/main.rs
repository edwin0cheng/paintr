macro_rules! L {
    ($str:literal) => {
        $crate::LocalizedString::new($str)
    };
}

mod commands;
mod dialogs;
mod menu;
mod tools;
mod ui;

use druid::{
    theme, AppDelegate, AppLauncher, Application, Color, Command, Data, DelegateCtx, Env, Handled,
    Lens, LocalizedString, Target, WindowDesc, WindowId,
};
use paintr_core::{
    actions::Paste, get_image_from_clipboard, put_image_to_clipboard, CanvasData, CopyMode,
    EditKind, UndoHistory,
};
use paintr_widgets::{theme_ext, widgets, EditorState};

use std::{
    path::{self, PathBuf},
    sync::Arc,
};

use dialogs::DialogData;
use tools::ToolKind;
use ui::ui_builder;
use widgets::notif_bar::Notification;

fn main() {
    let app_state = AppState {
        notifications: Arc::new(Vec::new()),
        modal: None,
        editor: EditorState {
            canvas: None,
            history: UndoHistory::new(),
            tool: ToolKind::Select,
            is_editing: false,
            cursor: None,
        },
    };

    let main_window = WindowDesc::new(ui_builder)
        .title(L!("paint-app-name"))
        .menu(menu::make_menu(&app_state))
        .window_size((800.0, 600.0));

    let user_l10n = find_user_l10n();

    let launcher = AppLauncher::with_window(main_window)
        .delegate(Delegate::default())
        .configure_env(|mut env, _| {
            env.set(theme::WINDOW_BACKGROUND_COLOR, Color::rgb8(0, 0x77, 0x88));
            theme_ext::init(&mut env);
        });

    let launcher = match user_l10n {
        Some(basedir) => launcher.localization_resources(
            vec!["builtin.ftl".to_string()],
            basedir.to_string_lossy().to_string(),
        ),
        None => launcher,
    };

    launcher.launch(app_state).expect("launch failed");
}

#[derive(Default, Debug)]
struct Delegate {
    windows: Vec<WindowId>,
}

type Error = Box<dyn std::error::Error>;

#[derive(Clone, Data, Lens, Debug)]
struct AppState {
    notifications: Arc<Vec<Notification>>,
    modal: Option<DialogData>,
    editor: EditorState<ToolKind>,
}

const NEW_FILE_NAME: &str = "Untitled";

fn to_rgba(img: image::DynamicImage) -> image::DynamicImage {
    image::DynamicImage::ImageRgba8(match img {
        image::DynamicImage::ImageRgba8(img) => img,
        _ => img.to_rgba8(),
    })
}

impl AppState {
    fn show_notification(&mut self, n: Notification) {
        Arc::make_mut(&mut self.notifications).push(n);
    }

    fn do_open_image(&mut self, path: &std::path::Path) -> Result<(), Error> {
        let img = image::open(path)?;
        self.editor.canvas = Some(CanvasData::new(path, to_rgba(img)));
        Ok(())
    }

    fn do_new_image_from_clipboard(&mut self) -> Result<(), Error> {
        let img = get_image_from_clipboard()?
            .ok_or_else(|| "Clipboard is empty / non-image".to_string())?;
        self.editor.canvas = Some(CanvasData::new(NEW_FILE_NAME, to_rgba(img)));
        Ok(())
    }

    fn do_new_image(&mut self, info: &dialogs::NewFileSettings) -> Result<(), Error> {
        let (w, h) = (
            info.width.expect("It must be valid after dialog closed."),
            info.height.expect("It must be valid after dialog closed."),
        );
        // Fill with white color
        let img = image::ImageBuffer::from_fn(w, h, |_, _| {
            image::Rgba([0xff_u8, 0xff_u8, 0xff_u8, 0xff_u8])
        });

        self.editor.canvas =
            Some(CanvasData::new(NEW_FILE_NAME, image::DynamicImage::ImageRgba8(img)));
        Ok(())
    }

    fn do_save_as_image(&mut self, path: &std::path::Path) -> Result<(), Error> {
        let canvas = self.editor.canvas.as_mut().ok_or_else(|| "No image was found.")?;
        canvas.save(path)?;
        Ok(())
    }

    fn do_copy(&mut self) -> Result<bool, Error> {
        let img = self.editor.canvas.as_ref().and_then(|canvas| {
            canvas.selection().map(|sel| sel.copy(canvas.merged(), CopyMode::Shrink))
        });

        let img = match img.flatten() {
            None => return Ok(false),
            Some(img) => img,
        };

        put_image_to_clipboard(&img)?;
        Ok(true)
    }

    fn do_paste(&mut self) -> Result<bool, Error> {
        let img = get_image_from_clipboard()?;
        let img = match img {
            Some(img) => img,
            None => return Ok(false),
        };
        let img = to_rgba(img);
        Ok(self.editor.do_edit(Paste::new(img), EditKind::NonMergeable))
    }

    fn image_file_name(&self) -> String {
        match &self.editor.canvas {
            None => NEW_FILE_NAME.to_owned(),
            Some(canvas) => canvas.path().to_string_lossy().into(),
        }
    }

    fn status(&self) -> Option<String> {
        Some(self.editor.canvas.as_ref()?.selection()?.description())
    }
}

impl Delegate {
    fn handle_command(
        &mut self,
        data: &mut AppState,
        ctx: &mut DelegateCtx,
        cmd: &druid::Command,
    ) -> Result<Handled, Error> {
        match cmd {
            _ if cmd.is(commands::FILE_EXIT_ACTION) => {
                ctx.submit_command(druid::commands::CLOSE_WINDOW);
            }
            _ if cmd.is(commands::FILE_NEW_ACTION) => {
                data.modal = Some(DialogData::new_file_settings());
                self.update_menu(data, ctx);
            }
            _ if cmd.is(commands::FILE_NEW_CLIPBOARD_ACTION) => {
                data.do_new_image_from_clipboard()?;
                data.show_notification(Notification::info("New file created"));
                self.update_menu(data, ctx);
            }
            _ if cmd.is(druid::commands::OPEN_FILE) => {
                let info = cmd.get_unchecked(druid::commands::OPEN_FILE);
                data.do_open_image(info.path())?;
                data.show_notification(Notification::info(format!(
                    "{} opened",
                    data.image_file_name()
                )));
                self.update_menu(data, ctx);
            }
            _ if cmd.is(druid::commands::SAVE_FILE_AS) => {
                let info = cmd.get_unchecked(druid::commands::SAVE_FILE_AS);
                data.do_save_as_image(info.path())?;
                data.show_notification(Notification::info(format!(
                    "{} saved",
                    data.image_file_name()
                )));
                self.update_menu(data, ctx);
            }
            _ if cmd.is(commands::EDIT_UNDO_ACTION) => {
                if let Some(desc) = data.editor.do_undo() {
                    data.show_notification(Notification::info(format!("Undo {}", desc)));
                }
            }
            _ if cmd.is(commands::EDIT_REDO_ACTION) => {
                if let Some(desc) = data.editor.do_redo() {
                    data.show_notification(Notification::info(format!("Redo {}", desc)));
                }
            }
            _ if cmd.is(commands::EDIT_COPY_ACTION) => {
                if data.do_copy()? {
                    data.show_notification(Notification::info("Copied"));
                }
            }
            _ if cmd.is(commands::EDIT_PASTE_ACTION) => {
                if data.do_paste()? {
                    data.show_notification(Notification::info("Pasted"));
                }
            }
            _ if cmd.is(commands::NEW_IMAGE_ACTION) => {
                let info = cmd.get_unchecked(commands::NEW_IMAGE_ACTION);
                data.do_new_image(info)?;
                data.show_notification(Notification::info("New file created"));
                self.update_menu(data, ctx);
            }
            _ if cmd.is(commands::ABOUT_TEST_ACTION) => {
                data.show_notification(Notification::info("Test"));
            }
            _ => return Ok(Handled::No),
        }

        Ok(Handled::Yes)
    }

    fn update_menu(&self, data: &AppState, ctx: &mut DelegateCtx) {
        let menu = menu::make_menu(data);

        for id in &self.windows {
            ctx.set_menu(menu.clone(), *id);
        }
    }
}

impl AppDelegate<AppState> for Delegate {
    fn command(
        &mut self,
        ctx: &mut DelegateCtx,
        _target: Target,
        cmd: &Command,
        data: &mut AppState,
        _env: &Env,
    ) -> Handled {
        let res = self.handle_command(data, ctx, cmd);

        match res {
            Err(err) => {
                data.show_notification(Notification::error(err.to_string()));
                Handled::Yes
            }
            Ok(it) => it,
        }
    }

    fn window_added(
        &mut self,
        id: WindowId,
        _data: &mut AppState,
        _env: &Env,
        _ctx: &mut DelegateCtx,
    ) {
        self.windows.push(id);
    }

    fn window_removed(
        &mut self,
        id: WindowId,
        _data: &mut AppState,
        _env: &Env,
        _ctx: &mut DelegateCtx,
    ) {
        if let Some(pos) = self.windows.iter().position(|x| *x == id) {
            self.windows.remove(pos);
        }

        // FIXME: Use commands::QUIT_APP
        // It do not works right now, maybe a druid bug
        Application::global().quit();
    }
}

fn find_user_l10n() -> Option<PathBuf> {
    let paths = vec![
        path::PathBuf::from("./resources/i18n/"),
        dirs::config_dir()?.join("paintr/resources/i18n/"),
    ];

    paths.into_iter().find(|it| it.exists())
}
