mod server;
mod twitch;

use std::fs;
use std::fs::read_to_string;
use std::path::PathBuf;
use directories_next::ProjectDirs;
use eframe::egui;
use eframe::egui::{Align, Color32, Context, FontId, Frame, Label, Layout, RichText, ScrollArea, Sense, Style, TextEdit, Theme, UiBuilder, Widget};
use serde_derive::{Deserialize, Serialize};
use tokio::runtime::Runtime;
use std::sync::mpsc::{Receiver, Sender};
use log::{error, info};
use tokio::process::Command;
use tokio::task::JoinSet;
use twitch_api::helix::Scope::{ChannelReadSubscriptions, UserReadFollows, UserReadSubscriptions};
use twitch_api::helix::streams::Stream;
use twitch_api::twitch_oauth2::{ClientId, ImplicitUserTokenBuilder};
use twitch_api::types::{CategoryId, TwitchCategory};
use url::Url;
use crate::server::PORT;
use crate::twitch::{check_login, get_followed_streams, get_streams, get_top_categories, TwitchError};
use crate::TwitchOption::{GetCategoryStreams, GetCategoryStreamsResult, GetFollowedStreams, GetFollowedStreamsResult, GetStreams, GetTopCategories, LoginResult, StreamsResult, TopCategoriesResult};

const CLIENT_ID: &str = "ualshng9w0vvyb4w8fql0z4dt3cz8k";

fn main() {
    env_logger::init();

    let rt = Runtime::new().expect("Unable to create Runtime");

    let _enter = rt.enter();

    // internal server for oauth
    std::thread::spawn(move || {
        rt.block_on(server::run())
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([800.0, 600.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "streamgui",
        options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);

            let app = App::default();

            Ok(Box::new(app))
        })
    ).expect("failed to render app");
}


#[derive(Deserialize, Serialize)]
struct AppConfig {
    token: Option<String>,
}


impl Default for AppConfig {
    fn default() -> Self {
        Self::load()
    }
}

impl AppConfig {
    fn save(&self) {

        match Self::get_path() {
            Some(path) => {
                let file_content = toml::to_string(&self).unwrap();
                fs::write(path, file_content).expect("Unable to save config file");
            }
            None => {
                panic!("Unable to find config file");
            }
        }
    }

    fn load() -> AppConfig {
        match Self::get_path() {
            Some(path) => {

                let file_contents = read_to_string(path).expect("failed to read config file");

                match toml::from_str::<AppConfig>(file_contents.as_str()) {
                    Ok(app_config) => {
                        app_config
                    }
                    Err(e) => {
                        panic!("failed to parse config file: {}", e);
                    }
                }
            }
            None => {
                panic!("Cannot get config path");
            }
        }
    }

    fn project_dirs() -> Option<ProjectDirs> {
        ProjectDirs::from("com", "porterca", "streamgui")
    }

    fn get_path() -> Option<PathBuf> {
        match Self::project_dirs() {
            Some(proj_dir) => {
                let mut config_path = proj_dir.config_dir().to_path_buf();

                if !config_path.exists() {
                    fs::create_dir_all(&config_path).unwrap_or_else(|_| panic!("failed to create config directory: {}", config_path.display()));
                }
                config_path.push("config.toml");

                if !config_path.exists() {
                    // use defaults
                    fs::File::create(config_path.clone()).expect("failed to create config file");
                }

                info!("Using config file: {}", config_path.display());
                Some(config_path)
            }
            None => {
                None
            }
        }
    }
}

enum AppView {
    Login,
    Categories,
    Streams,
    FollowedLive,
    Settings,
    CategoryView,
}

enum TwitchOption {
    LoginCheck,
    LoginResult(bool),
    GetTopCategories(Option<String>),
    GetStreams(Option<String>),
    GetFollowedStreams,
    GetCategoryStreams(CategoryId),
    TopCategoriesResult(Result<Vec<TwitchCategory>, TwitchError>),
    StreamsResult(Result<Vec<Stream>, TwitchError>),
    GetFollowedStreamsResult(Result<Vec<Stream>, TwitchError>),
    GetCategoryStreamsResult(Result<Vec<Stream>, TwitchError>),
}

struct TwitchMessage {
    token: Option<String>,
    opt: TwitchOption,
}

struct App {
    token: String,
    config: AppConfig,
    login_pending: bool,
    current_view: AppView,
    error_message: Option<String>,
    categories: Option<Vec<TwitchCategory>>,
    streams: Option<Vec<Stream>>,
    followed_streams: Option<Vec<Stream>>,
    focused_stream: Option<Stream>,
    focused_category: Option<TwitchCategory>,
    focused_category_streams: Option<Vec<Stream>>,
    send: Sender<TwitchMessage>,
    recv: Receiver<TwitchMessage>,
    streamlink_tasks: JoinSet<()>,
}

impl Default for App {
    fn default() -> Self {
        let (send, recv) = std::sync::mpsc::channel();
        let config = AppConfig::default();

        Self {
            token: config.token.clone().unwrap_or_default(),
            config: config,
            login_pending: true,
            current_view: AppView::Login,
            error_message: None,
            categories: None,
            streams: None,
            followed_streams: None,
            focused_stream: None,
            focused_category: None,
            focused_category_streams: None,
            send,
            recv,
            streamlink_tasks: JoinSet::new(),
        }
    }
}

fn setup_style(style: &mut Style) {
    // don't have all text be selectable
    style.interaction.selectable_labels = false;
}

impl App {
    fn logout(&mut self) {
        self.token = "".to_string();
        self.config.token = None;
        self.config.save();
        self.login_pending = false;
        self.current_view = AppView::Login;
        self.categories = None;
        self.streams = None;
        self.followed_streams = None;
        self.focused_stream = None;
        self.focused_category = None;
        self.focused_category_streams = None;
    }

    fn start_stream(&mut self, stream: Stream) {
        self.streamlink_tasks.spawn(async move {
            let _child = Command::new("streamlink")
                .arg("--twitch-low-latency")
                .arg(format!("https://twitch.tv/{}", stream.user_name))
                .arg("best")
                .spawn();
        });
    }

    fn request_streams(&mut self, ctx: Option<Context>) {
        info!("Requesting streams");
        let req = TwitchMessage{
            token: Option::from(self.token.clone()),
            opt: GetStreams(None)
        };
        send_req(req, self.send.clone(), ctx);
    }

    fn request_categories(&mut self, ctx: Option<Context>) {
        let req = TwitchMessage{token: Option::from(self.token.clone()), opt:
        GetTopCategories(None)};
        send_req(req, self.send.clone(), ctx);
    }

    fn request_followed(&mut self, ctx: Option<Context>) {
        let req = TwitchMessage{token: Option::from(self.token.clone()), opt:
        GetFollowedStreams};
        send_req(req, self.send.clone(), ctx);
    }
}


impl eframe::App for App {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        if let Ok(added) = self.recv.try_recv() {
            let TwitchMessage { opt, ..} = added;
            match opt {
                LoginResult(result) => {
                    if let AppView::Login = self.current_view {
                        if result {
                            self.error_message = None;
                            self.current_view = AppView::FollowedLive;
                            self.config.token = Some(self.token.clone());
                            self.config.save();

                            self.request_followed(Some(ctx.clone()))
                        } else {
                            self.error_message = Option::from("login failed".to_owned());
                        }
                    }
                }
                TopCategoriesResult(result) => {
                    self.categories = Some(result.unwrap());
                }
                StreamsResult(result) => {

                    self.streams = Some(result.unwrap());
                }
                GetFollowedStreamsResult(result) => {
                    self.followed_streams = Some(result.unwrap());
                }
                GetCategoryStreamsResult(result) => {
                    self.focused_category_streams = Some(result.unwrap());
                }

                _ => {
                    error!("Received unexpected message");
                }
            }
        }

        ctx.set_pixels_per_point(1.5);
        ctx.style_mut_of(Theme::Dark, setup_style);

        if self.login_pending && !self.token.is_empty() {
            egui::panel::CentralPanel::default().show(ctx, |ui| {
                ui.spinner();
            });
            // auto login
            let req = TwitchMessage{
                token: Option::from(self.token.clone()),
                opt: TwitchOption::LoginCheck
            };
            send_req(req, self.send.clone(), Some(ctx.clone()));
            self.login_pending = false;
            return;
        } else {
            self.login_pending = false;
        }


        egui::SidePanel::left("side_panel").show(ctx, |ui| {
            ui.heading("streamgui");

            ui.separator();

            if let AppView::Login = self.current_view {
                return;
            }

            ui.heading("Browse");

            if ui.button("Categories").clicked() {
                self.current_view = AppView::Categories;
                self.request_categories(Some(ctx.clone()));
            }
            if ui.button("Streams").clicked() {
                self.current_view = AppView::Streams;
                self.request_streams(Some(ctx.clone()));
            }

            ui.separator();
            ui.heading("Followed");

            if ui.button("Live").clicked() {
                self.current_view = AppView::FollowedLive;
                self.request_followed(Some(ctx.clone()));
            }

            ui.separator();

            if ui.button("Settings").clicked() {
                self.current_view = AppView::Settings;
            }

            if ui.button("Logout").clicked() {
                self.logout();
            }
        });

        if self.error_message.is_some() {
            egui::TopBottomPanel::bottom("bottom_panel").resizable(false).show(ctx, |ui| {
                let msg = self.error_message.clone();
                ui.label(RichText::new(msg.unwrap().as_str
                ()).font(FontId::proportional(20.0)).color(Color32::RED));
            });
        }

        if self.focused_stream.is_some() {
            egui::SidePanel::right("stream_panel").show(ctx, |ui| {
                if ui.button("Close").clicked() {
                    self.focused_stream = None;
                    return;
                }

                let stream = self.focused_stream.clone().unwrap();
                ui.heading(stream.title.as_str());
                ui.heading(stream.user_name.as_str());
                ui.separator();
                if ui.button("Watch").clicked() {
                    self.start_stream(stream);
                }
            });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.with_layout(Layout::top_down(Align::Min), |ui| {

                match self.current_view {
                    AppView::Login => {
                        ui.heading("Login");
                        ui.label("Opens a browser to authorize streamgui with Twitch. Paste the \
                        token from the page into the box and then log in.");
                        if ui.button("Open browser").clicked() {

                            let client_id = ClientId::new(CLIENT_ID.to_owned());

                            let redirect_url = Url::parse(format!
                            ("http://localhost:{PORT}").as_str()).expect("Invalid redirect url");


                            let mut builder = ImplicitUserTokenBuilder::new(client_id,
                                                                            redirect_url)
                                .set_scopes(vec!(ChannelReadSubscriptions, UserReadFollows,
                                                 UserReadSubscriptions));

                            let (url, _csrf_token) = builder.generate_url();

                            open::that(url.as_str()).expect("failed to open browser");
                        }
                        ui.label("paste token:");
                        ui.add(TextEdit::singleline(&mut self.token).password(true));
                        if ui.button("Login").clicked() {
                            let req = TwitchMessage{
                                token: Option::from(self.token.clone()),
                                opt: TwitchOption::LoginCheck
                            };
                            send_req(req, self.send.clone(), Some(ctx.clone()));
                        }
                    },
                    AppView::Categories => {
                        ui.heading("Categories");
                        if ui.button("🔄").clicked() {
                            // TODO resend categories request
                        }

                        let scroll_area = ScrollArea::vertical();

                        scroll_area.show_rows(ui, 100.0, self
                            .categories.iter().len(), |ui, _row_range| {
                            for category in self.categories.iter().flatten() {

                                let category_button = ui.scope_builder(
                                    UiBuilder::new().id_salt(category.id.to_string()).sense(Sense::click()),
                                    |ui| {
                                        let response = ui.response();
                                        let visuals = ui.style().interact(&response);
                                        let text_color = visuals.text_color();

                                        Frame::canvas(ui.style())
                                            .fill(visuals.bg_fill)
                                            .stroke(visuals.bg_stroke)
                                            .inner_margin(ui.spacing().menu_margin)
                                            .show(ui, |ui| {
                                                ui.set_width(ui.available_width());
                                                ui.add_space(20.0);
                                                ui.vertical_centered(|ui| {
                                                    Label::new(
                                                        RichText::new(category.name.as_str())
                                                            .color(text_color)
                                                            .size(20.0)
                                                    )
                                                        .selectable(false)
                                                        .ui(ui);
                                                })
                                            });
                                    }
                                );

                                if category_button.response.clicked() {
                                    self.current_view = AppView::CategoryView;
                                    self.focused_category = Some(category.clone());
                                    self.focused_category_streams = None;

                                    let req = TwitchMessage {
                                        token: Option::from(self.token.clone()),
                                        opt: GetCategoryStreams(category.id.clone()),
                                    };
                                    send_req(req, self.send.clone(), Some(ctx.clone()));
                                }
                            }
                        });
                    },
                    AppView::Streams => {
                        ui.heading("Streams");
                        if ui.button("🔄").clicked() {
                            self.request_streams(Some(ctx.clone()));
                        }

                        let scroll_area = ScrollArea::vertical();

                        scroll_area.show_rows(ui, 100.0, self
                            .streams.iter().len(), |ui, _row_range| {
                            for stream in self.streams.iter().flatten() {
                                if ui.button(stream.title.as_str()).clicked() {
                                    self.focused_stream = Option::from(stream.clone());
                                }
                                ui.label(stream.user_name.as_str());
                            }
                        });
                    },
                    AppView::FollowedLive => {
                        ui.heading("Followed Live");

                        let scroll_area = ScrollArea::vertical();

                        scroll_area.show_rows(ui, 100.0, self
                            .followed_streams.iter().len(), |ui, _row_range| {
                            for stream in self.followed_streams.iter().flatten() {
                                if ui.button(stream.title.as_str()).clicked() {
                                    self.focused_stream = Option::from(stream.clone());
                                }
                                ui.label(stream.user_name.as_str());
                            }
                        });
                    },
                    AppView::Settings => {
                        ui.heading("Settings");
                    }
                    AppView::CategoryView => {

                        if self.focused_category.is_none() {
                            // go back if no focused category
                            self.current_view = AppView::Categories;
                            return;
                        }

                        ui.horizontal(|ui| {
                            if ui.button("⬅").clicked() {
                                self.current_view = AppView::Categories;
                            }
                            if ui.button("🔄").clicked() {
                                // TODO resend category streams request
                            }
                        });

                        let category = self.focused_category.clone().unwrap();

                        ui.heading(category.name.as_str());
                        ui.separator();

                        let scroll_area = ScrollArea::vertical();
                        scroll_area.show_rows(ui, 100.0, self.focused_category_streams.iter().len
                        (), |ui, _row_range| {
                            for stream in self.focused_category_streams.iter().flatten() {
                                if ui.button(stream.title.as_str()).clicked() {
                                    self.focused_stream = Option::from(stream.clone());
                                }
                                ui.label(stream.user_name.as_str());
                            }

                        });
                    }
                }
            })
        });
    }
}

fn send_req(msg: TwitchMessage, tx: Sender<TwitchMessage>, ctx: Option<Context>) {
    tokio::spawn(async move {
        if msg.token.is_none() {
            error!("Missing token on message");
            return;
        }

        let token = msg.token.unwrap();

        match msg.opt {
            TwitchOption::LoginCheck => {

                let result = check_login(token).await;
                let resp = TwitchMessage {
                    token: None,
                    opt: LoginResult(result),
                };
                tx.send(resp).expect("Failed to send resp");
            }
            GetTopCategories(pagination) => {
                let result = get_top_categories(token, pagination).await;

                let resp = TwitchMessage {
                    token: None,
                    opt: TopCategoriesResult(result),
                };
                tx.send(resp).expect("Failed to send resp");
            }
            GetStreams(pagination) => {
                let result = get_streams(token, None, pagination).await;

                let resp = TwitchMessage {
                    token: None,
                    opt: StreamsResult(result),
                };
                tx.send(resp).expect("Failed to send resp");
            },
            GetFollowedStreams => {
                let result = get_followed_streams(token, None).await;

                let resp = TwitchMessage {
                    token: None,
                    opt: GetFollowedStreamsResult(result),
                };
                tx.send(resp).expect("Failed to send resp");
            },
            GetCategoryStreams(category) => {
                let result = get_streams(token, Some(category.clone()), None).await;

                let resp = TwitchMessage {
                    token: None,
                    opt: GetCategoryStreamsResult(result),
                };
                tx.send(resp).expect("Failed to send resp");
            }
            _ => {}
        }

        if let Some(ctx) = ctx {
            ctx.request_repaint();
        }
    });
}
