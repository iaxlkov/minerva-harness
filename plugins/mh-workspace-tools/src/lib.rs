//! Native workspace tool plugin and implementations.
pub use mh_tool_runtime::*;
pub use mh_tool_runtime as tools;
use mh_context::ContextService;
use cordis::{Context, Plugin, Service};
use serde_json::json;
use std::{convert::Infallible, sync::Arc};
mod native;
mod process_tree;
pub use native::{register_native, save_attachment, EDIT_LOCK, atomic_write};
pub struct WorkspaceTools;
impl Service for WorkspaceTools {
    const NAME: &'static str = "studio.workspace-tools";
}
pub struct WorkspaceToolsPlugin(pub bool);
fn apply_error(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(error.to_string())
}
impl Plugin for WorkspaceToolsPlugin {
    type Config = ();
    type Input = ();
    type PrepareError = Infallible;
    type ApplyError = std::io::Error;
    fn prepare(&self, (): ()) -> Result<(), Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: Context, _: &()) -> Result<(), std::io::Error> {
        if !self.0 {
            return Ok(());
        }
        let tools = ctx.try_service::<ToolRegistry>().map_err(apply_error)?;
        let context = ctx.try_service::<ContextService>().map_err(apply_error)?;
        let owned = tools.clone();
        let _cleanup = ctx
            .effect_sync(move || {
                owned.revoke_owner("native.workspace");
                owned.revoke_owner("native.interaction");
                owned.revoke_owner("native.attachments");
            })
            .map_err(apply_error)?;
        crate::register_native(&tools).map_err(apply_error)?;
        for (name, description, properties, required, side_effect) in [
            ("read_attachment", "Read a session attachment in bounded byte chunks. UTF-8 text is returned directly; binary or split UTF-8 chunks use base64. Continue at nextOffset until eof.", json!({"id":{"type":"string"},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":1024}}), vec!["id"], false),
            ("save_attachment", "Copy a session attachment byte-for-byte to a new workspace file for processing (PDF, archive, spreadsheet, etc.). Never overwrites existing files.", json!({"id":{"type":"string"},"path":{"type":"string","minLength":1}}), vec!["id","path"], true),
        ] {
            let context = context.clone();
            tools.register(if name=="read_attachment" {"native.attachments"} else {"native.workspace"}, crate::tools::ToolDefinition {
                name:name.into(), description:description.into(),
                input_schema:json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
                output_schema:json!({"type":"object"}), side_effect,
            }, move |ctx,args| {
                let context=context.clone();
                Box::pin(async move {
                    let id=args["id"].as_str().ok_or("missing attachment id")?;
                    if name=="read_attachment" {
                        context.read_attachment(&ctx.session_id,id,args["offset"].as_u64().unwrap_or(0) as usize,args["limit"].as_u64().unwrap_or(1024) as usize)
                    } else {
                        let attachment=context.get_attachment(&ctx.session_id,id)?;
                        crate::save_attachment(&ctx,args["path"].as_str().ok_or("missing path")?,&attachment.data)
                    }
                })
            }).map_err(apply_error)?;
        }
        tools.register("native.workspace", crate::tools::ToolDefinition {
            name: "load_skill".into(),
            description: "Load the instructions of an available workspace skill by name".into(),
            input_schema: json!({"type":"object","properties":{"name":{"type":"string","minLength":1}},"required":["name"],"additionalProperties":false}),
            output_schema: json!({"type":"object","properties":{"instructions":{"type":"string"}},"required":["instructions"]}),
            side_effect: false,
        }, move |ctx, args| {
            let context = context.clone();
            Box::pin(async move { Ok(json!({"instructions":context.load_skill(&ctx.session_id, args["name"].as_str().ok_or("missing skill name")?)?})) })
        }).map_err(apply_error)?;
        let _publication = ctx.provide(Arc::new(WorkspaceTools)).map_err(apply_error)?;
        Ok(())
    }
}
