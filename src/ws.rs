use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use uuid::Uuid;

use crate::state::{AppState, ServerRuntime};

pub async fn console_ws(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(state, id, socket))
}

async fn handle_socket(state: AppState, id: Uuid, mut socket: WebSocket) {
    // IMPORTANT: always subscribe to the *same* per-server runtime entry that
    // `process::start_server` uses (it does `.entry(id).or_insert_with(...)`
    // too). Previously, opening the console before ever starting the server
    // fell back to a dummy/unrelated channel and silently never received
    // anything - the "works only after switching tabs and back" bug, since a
    // fresh WS connection on the second visit would by then find the real
    // entry. Creating the entry here (write lock) instead of only reading it
    // guarantees both sides always share the same broadcast channel.
    let (backlog, mut rx) = {
        let mut runtime = state.runtime.write().await;
        let rt = runtime.entry(id).or_insert_with(ServerRuntime::default);
        (rt.backlog.clone(), rt.tx.subscribe())
    };

    for line in backlog {
        if socket.send(Message::Text(line)).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            line = rx.recv() => {
                match line {
                    Ok(line) => {
                        if socket.send(Message::Text(line)).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let _ = crate::process::send_command(&state, id, &text).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
}

pub async fn playit_ws(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_playit_socket(state, socket))
}

async fn handle_playit_socket(state: AppState, mut socket: WebSocket) {
    // Subscribe first, then replay the backlog: this ordering (matching
    // console_ws above) avoids a gap where a line printed between the
    // backlog snapshot and the subscribe call would be missed entirely.
    let mut rx = state.playit_tx.subscribe();
    let backlog = state.playit_backlog.read().await.clone();
    for line in backlog {
        if socket.send(Message::Text(line)).await.is_err() {
            return;
        }
    }

    while let Ok(line) = rx.recv().await {
        if socket.send(Message::Text(line)).await.is_err() {
            break;
        }
    }
}
