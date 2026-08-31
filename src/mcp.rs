//! `keyhold mcp` — the process a coding agent talks to.
//!
//! It speaks the Model Context Protocol over stdin and stdout, and forwards everything to the
//! running keyhold through its unix socket. **It holds no key, no vault and no decision.** If the
//! vault is closed, this process can do nothing at all: there is no socket to connect to, and it
//! says so rather than pretending.
//!
//! The tool descriptions are written for a model to read, and they say two things on purpose: that
//! `keyhold_run` is preferred over `keyhold_get`, and that a value which comes back must not be
//! written down anywhere. The first is a real control — the value never enters the transcript. The
//! second is advice, and the interface never relies on it: what protects the vault is that the
//! person approves each read and can see afterwards exactly what was taken.

use std::io::{BufRead, Write};
use std::os::unix::net::UnixStream;

use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::agent::protocol::{Request, Response};
use crate::agent::socket_path;

/// The protocol version this speaks. Clients that ask for a different one still get this.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// Ask the daemon one question and read one answer.
///
/// A connection per request rather than one kept open: an agent's questions arrive seconds or
/// minutes apart, and a socket held open across that is a socket that outlives the vault being
/// locked.
fn ask(request: &Request) -> Result<Response> {
    let path = socket_path();

    let mut stream = UnixStream::connect(&path).map_err(|_| {
        anyhow::anyhow!(
            "o keyhold não está aberto nesta máquina (nenhum socket em {})",
            path.display()
        )
    })?;

    let line = format!("{}\n", serde_json::to_string(request)?);
    stream.write_all(line.as_bytes())?;
    stream.flush()?;

    let mut answer = String::new();
    std::io::BufReader::new(&stream).read_line(&mut answer)?;

    if answer.trim().is_empty() {
        bail!("o keyhold fechou a conexão sem responder");
    }

    Ok(serde_json::from_str(&answer)?)
}

/// What the tools look like to an agent.
fn tools() -> Value {
    json!([
        {
            "name": "keyhold_search",
            "description":
                "Encontra registros no cofre local pelo título, usuário ou endereço. Devolve apenas \
                 metadados — nunca uma senha ou o valor de um campo. Use isto primeiro para achar o \
                 id de um registro.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Parte do título, do usuário ou do endereço. Vazio lista tudo o que está ao alcance."
                    }
                }
            }
        },
        {
            "name": "keyhold_describe",
            "description":
                "Lista os campos de um registro pelo rótulo e pelo tipo, sem os valores. Use para \
                 saber o nome exato do campo antes de pedir um valor.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "record": { "type": "string", "description": "O id do registro." }
                },
                "required": ["record"]
            }
        },
        {
            "name": "keyhold_run",
            "description":
                "PREFIRA ESTA FERRAMENTA. Executa um comando com o segredo em uma variável de \
                 ambiente do processo filho e devolve a saída. O valor do segredo nunca entra nesta \
                 conversa. Exemplo: rodar `psql \"$DB_URL\"` com o campo Senha em DB_URL. A pessoa \
                 aprova o pedido na janela do keyhold.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "record": { "type": "string", "description": "O id do registro." },
                    "field": { "type": "string", "description": "O rótulo do campo, como em keyhold_describe." },
                    "command": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "O comando e seus argumentos, já separados."
                    },
                    "variable": {
                        "type": "string",
                        "description": "Nome da variável de ambiente que receberá o segredo. Padrão: KEYHOLD_SECRET."
                    }
                },
                "required": ["record", "field", "command"]
            }
        },
        {
            "name": "keyhold_get",
            "description":
                "Devolve o valor de um campo. Use apenas quando keyhold_run não resolver, porque o \
                 valor passa a fazer parte desta conversa e sai desta máquina. NUNCA escreva o valor \
                 em um arquivo, commit, log, mensagem ou resumo: use-o na chamada imediata e \
                 descarte. A pessoa aprova cada pedido na janela do keyhold.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "record": { "type": "string", "description": "O id do registro." },
                    "field": { "type": "string", "description": "O rótulo do campo, como em keyhold_describe." }
                },
                "required": ["record", "field"]
            }
        }
    ])
}

/// Turn a daemon answer into the text an agent reads.
fn render(response: Response) -> Value {
    let (text, is_error) = match response {
        Response::Status { unlocked, vaults, records } => (
            format!(
                "O cofre está {}. {} cofre(s) ao alcance de um agente, com {records} registro(s).",
                if unlocked { "aberto" } else { "trancado" },
                vaults.len()
            ),
            false,
        ),
        Response::Records { records } => {
            if records.is_empty() {
                ("Nenhum registro ao alcance corresponde a essa busca.".to_string(), false)
            } else {
                let lines: Vec<String> = records
                    .iter()
                    .map(|record| {
                        format!(
                            "- {} (id: {}, cofre: {}, tipo: {}){}{}",
                            record.title,
                            record.id,
                            record.vault,
                            record.kind,
                            if record.username.is_empty() {
                                String::new()
                            } else {
                                format!(", usuário: {}", record.username)
                            },
                            if record.url.is_empty() {
                                String::new()
                            } else {
                                format!(", endereço: {}", record.url)
                            }
                        )
                    })
                    .collect();

                (lines.join("\n"), false)
            }
        }
        Response::Fields { title, fields } => {
            let lines: Vec<String> = fields
                .iter()
                .map(|field| {
                    format!(
                        "- {} ({}{})",
                        field.label,
                        field.kind,
                        if field.secret { ", precisa de aprovação" } else { "" }
                    )
                })
                .collect();

            (format!("{title}:\n{}", lines.join("\n")), false)
        }
        Response::Value { value } => (
            format!(
                "{value}\n\n[keyhold] Este valor saiu do cofre local e agora faz parte desta \
                 conversa. Use-o só na chamada imediata; não o escreva em arquivo, commit, log ou \
                 resumo."
            ),
            false,
        ),
        Response::Ran { code, stdout, stderr } => (
            format!(
                "O comando terminou com código {code}.\n\n--- saída ---\n{stdout}\n--- erros ---\n{stderr}"
            ),
            code != 0,
        ),
        Response::Refused { reason } => (format!("[keyhold] {reason}"), true),
    };

    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error
    })
}

fn call(name: &str, arguments: &Value) -> Result<Value> {
    let text = |key: &str| -> String {
        arguments.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
    };

    let request = match name {
        "keyhold_search" => Request::Search { query: text("query") },
        "keyhold_describe" => Request::Describe { record: text("record") },
        "keyhold_get" => Request::Get { record: text("record"), field: text("field") },
        "keyhold_run" => Request::Run {
            record: text("record"),
            field: text("field"),
            command: arguments
                .get("command")
                .and_then(Value::as_array)
                .map(|items| {
                    items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect()
                })
                .unwrap_or_default(),
            variable: text("variable"),
        },
        other => bail!("ferramenta desconhecida: {other}"),
    };

    Ok(render(ask(&request)?))
}

/// Read requests from stdin and answer on stdout, until the client goes away.
pub fn serve() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;

        if line.trim().is_empty() {
            continue;
        }

        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        // A notification has no id and expects no answer. Replying to one is a protocol error.
        let Some(id) = message.get("id").cloned() else {
            continue;
        };

        let method = message.get("method").and_then(Value::as_str).unwrap_or_default();
        let params = message.get("params").cloned().unwrap_or(json!({}));

        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "keyhold", "version": env!("CARGO_PKG_VERSION") }
            })),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

                call(name, &arguments)
            }
            "ping" => Ok(json!({})),
            other => Err(anyhow::anyhow!("método não suportado: {other}")),
        };

        let response = match result {
            Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
            Err(error) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32000, "message": error.to_string() }
            }),
        };

        writeln!(stdout, "{response}")?;
        stdout.flush()?;
    }

    Ok(())
}
