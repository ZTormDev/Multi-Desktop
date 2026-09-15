//! Publishes the PipeWire node exported by this isolated Gamescope session.
use serde_json::Value;
use std::{
    env, fs, io, os::unix::fs::PermissionsExt, path::Path, process::Command, thread, time::Duration,
};

fn main() -> io::Result<()> {
    let runtime = env::var("MULTIDESKTOP_DESKTOP_RUNTIME")
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "missing desktop runtime"))?;
    let status_path = Path::new(&runtime).join("capture-node");
    loop {
        let status = match discover_gamescope_node() {
            Ok(Some(node_id)) => format!("state=ready\nnode_id={node_id}\n"),
            Ok(None) => "state=waiting\n".to_owned(),
            Err(error) => format!("state=error\nreason={}\n", sanitize(&error.to_string())),
        };
        write_status(&status_path, &status)?;
        thread::sleep(Duration::from_secs(1));
    }
}

fn discover_gamescope_node() -> io::Result<Option<u64>> {
    let output = Command::new("pw-dump").output()?;
    if !output.status.success() {
        return Err(io::Error::other("pw-dump could not query PipeWire"));
    }
    let objects: Vec<Value> = serde_json::from_slice(&output.stdout)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "pw-dump returned invalid JSON"))?;
    Ok(find_gamescope_node(&objects))
}

fn find_gamescope_node(objects: &[Value]) -> Option<u64> {
    objects.iter().find_map(|object| {
        let is_node = object.get("type").and_then(Value::as_str) == Some("PipeWire:Interface:Node");
        let properties = object.pointer("/info/props")?.as_object()?;
        let name = properties.get("node.name")?.as_str()?.to_ascii_lowercase();
        let media_class = properties
            .get("media.class")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        if is_node && name.contains("gamescope") && media_class.contains("video") {
            object.get("id").and_then(Value::as_u64)
        } else {
            None
        }
    })
}

fn write_status(path: &Path, contents: &str) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, contents)?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    fs::rename(temporary, path)
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .filter(|character| *character != '\n' && *character != '\r')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_only_gamescope_video_node() {
        let objects: Vec<Value> = serde_json::from_str(
            r#"[
              {"id":7,"type":"PipeWire:Interface:Node","info":{"props":{"node.name":"camera","media.class":"Video/Source"}}},
              {"id":42,"type":"PipeWire:Interface:Node","info":{"props":{"node.name":"gamescope-stream","media.class":"Video/Source"}}}
            ]"#,
        ).unwrap();
        assert_eq!(find_gamescope_node(&objects), Some(42));
    }
}
