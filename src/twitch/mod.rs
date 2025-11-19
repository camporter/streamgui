use std::fmt;
use std::option::Option;
use twitch_api::helix::games::{GetTopGamesRequest};
use twitch_api::helix::{ClientRequestError, Cursor, Paginated};
use twitch_api::helix::streams::{GetFollowedStreamsRequest, GetStreamsRequest, Stream};
use twitch_api::twitch_oauth2::{AccessToken, TwitchToken, UserToken};
use twitch_api::TwitchClient;
use twitch_api::types::{CategoryId, Collection, TwitchCategory};


#[derive(Debug)]
pub enum TwitchError {
    ClientError(ClientRequestError<reqwest::Error>),
    TokenError,
    UserIdError,
}

impl fmt::Display for TwitchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TwitchError")
    }
}

pub async fn check_login(token: String) -> bool {
    let client: TwitchClient<reqwest::Client> = TwitchClient::new();
    match get_token(client, token).await {
        Ok(_) => {
            true
        }
        Err(_) => {
            false
        }
    }

}

pub async fn get_token(client: TwitchClient<'static, reqwest::Client>, token: String) ->
                                                                                 Result<UserToken, TwitchError> {
    let token = UserToken::from_existing(
        &client,
        AccessToken::new(token),
        None,
        None
    ).await;

    match token {
        Ok(token) => {
           Ok(token)
        },
        Err(_) => {
            Err(TwitchError::TokenError)
        }
    }
}

pub async fn get_top_categories(token: String, pagination: Option<String>) ->
                                                                           Result<Vec<TwitchCategory>, TwitchError> {

    let client: TwitchClient<reqwest::Client> = TwitchClient::new();

    let token = get_token(client.clone(), token).await?;

    let mut req = GetTopGamesRequest::default().first(50);

    if let Some(pagination) = pagination {
        req.set_pagination(Some(Cursor::new
            (pagination)));
    }

    let result = client.helix.req_get(req, &token).await;

    match result {
        Ok(resp) => {
            Ok(resp.data)
        }
        Err(err) => {
            Err(TwitchError::ClientError(err))
        }
    }
}

pub async fn get_streams(token: String, game_id: Option<CategoryId>, pagination: Option<String>) -> Result<Vec<Stream>, TwitchError> {
    let client: TwitchClient<reqwest::Client> = TwitchClient::new();

    let token = get_token(client.clone(), token).await?;

    let mut req = GetStreamsRequest::default().first(50);

    if let Some(gid) = game_id {
        req.game_id = Collection::from(vec!(gid));
    }

    if let Some(pagination) = pagination {
        req.set_pagination(Some(Cursor::new(pagination)));
    }

    let result = client.helix.req_get(req, &token).await;

    match result {
        Ok(resp) => {
            Ok(resp.data)
        }
        Err(err) => {
            Err(TwitchError::ClientError(err))
        }
    }
}

pub async fn get_followed_streams(token: String, pagination: Option<String>) ->
                                                                             Result<Vec<Stream>,
                                                                                 TwitchError> {
    let client: TwitchClient<reqwest::Client> = TwitchClient::new();

    let token = get_token(client.clone(), token).await?;

    let user_id = token.user_id().clone().ok_or(TwitchError::UserIdError)?;

    let mut req = GetFollowedStreamsRequest::user_id(user_id).first(50);

    if let Some(pagination) = pagination {
        req.set_pagination(Some(Cursor::new(pagination)));
    }

    let result = client.helix.req_get(req, &token).await;

    match result {
        Ok(resp) => {
            Ok(resp.data)
        }
        Err(err) => {
            Err(TwitchError::ClientError(err))
        }
    }
}
