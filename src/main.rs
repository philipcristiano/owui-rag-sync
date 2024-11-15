use clap::Parser;
use s3::creds::Credentials;
use s3::error::S3Error;
use s3::Region;
use s3::{Bucket, BucketConfiguration};
use serde::{Deserialize, Serialize};
use std::str;

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(short, long, default_value = "bucket")]
    bucket: String,
    #[arg(short, long, default_value = "http://localhost:9000")]
    endpoint: String,
    #[arg(short, long, default_value = "eu-central-1")]
    region: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // This requires a running minio server at localhost:9000
    let args = Args::parse();

    let region = Region::Custom {
        region: args.region,
        endpoint: args.endpoint,
    };
    let credentials = Credentials::default()?;

    let bucket =
        Bucket::new(args.bucket.as_str(), region.clone(), credentials.clone())?.with_path_style();

    let since = chrono::Utc::now() - chrono::Duration::hours(48);

    let items = bucket.list("".to_string(), None).await?;
    for item in items {
        for f in item.contents {
            let f_modified = chrono::DateTime::parse_from_rfc3339(&f.last_modified)?;
            println!("{:?}", f);
            println!("{:?}", f.key);
            if f_modified < since {
                println!("Modified before `since`, skipping");
                continue;
            }
            if f.key.ends_with(".md") {
                let safe_name = str::replace(f.key.as_str(), "/", "-");
                let safe_name = str::replace(safe_name.as_str(), " ", "-");
                println!("{:?}", safe_name);
                let obj = bucket.get_object(f.key.clone()).await?;
                let b = obj.to_vec().to_owned();
                if b.len() > 0 {
                    send_as_file(safe_name, b).await?;
                }
            }
        }
    }

    Ok(())
}

#[derive(Deserialize, Serialize, Debug)]
struct UploadFileResponse {
    id: String,
}
#[derive(Deserialize, Serialize, Debug)]
struct AddFileToKnowledgeBase {
    file_id: String,
}

use std::env;

async fn send_as_file(name: String, data: Vec<u8>) -> anyhow::Result<()> {
    use reqwest::multipart;

    let form = multipart::Form::new();
    let file_part = multipart::Part::bytes(data).file_name(name.clone());
    let form = form.part("file", file_part);
    let client = reqwest::Client::new();
    let owui_url = env::var("OPENWEBUI_URL").expect("env OPENWEBUI_URL must be set");
    let owui_token = env::var("OPENWEBUI_BEARER_TOKEN").expect("env OPENWEBUI_BEARER_TOKEN must be set");
    let owui_kid = env::var("OPENWEBUI_KNOWLEDGE_ID").expect("env OPENWEBUI_KNOWLEDGE_ID must be set");
    let file_url = format!("{owui_url}/api/v1/files/");
    let knowledge_url = format!("{owui_url}/api/v1/knowledge/{owui_kid}/file/add");
    let bearer = format!("Bearer {owui_token}");

    println!("Uploading file: {:?} to URL {:?} ", name, file_url);
    let resp = client
        .post(file_url)
        .header("Authorization", bearer.clone())
        .multipart(form)
        .send()
        .await?;
    let text = resp.text().await?;
    let json: UploadFileResponse = serde_json::from_str(&text)?;
    let add = AddFileToKnowledgeBase { file_id: json.id };
    println!("request: {:?}", add);
    let resp = client
        .post(knowledge_url)
        .header("Authorization", bearer)
        .header("Content-Type", "application/json")
        .json(&add)
        .send()
        .await?;
    println!("Response: {}", resp.text().await?);

    Ok(())
}
