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
    #[arg(long, default_value = "24")]
    hours_since_modified: u32,
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

    let since = chrono::Utc::now() - chrono::Duration::hours(args.hours_since_modified.into());

    println!("Getting existing files");
    let mut uploaded_files_info = get_uploaded_files_info().await?;
    uploaded_files_info.sort_by_key(|fi| fi.filename.clone());

    let items = bucket.list("".to_string(), None).await?;
    for item in items {
        for f in item.contents {
            let f_modified = chrono::DateTime::parse_from_rfc3339(&f.last_modified)?;
            println!("{:?}", f.key);
            if f_modified < since {
                println!("Modified before `since`, skipping");
                continue;
            }
            if f.key.ends_with(".md") {
                let safe_name = str::replace(f.key.as_str(), "/", "-");
                let safe_name = str::replace(safe_name.as_str(), " ", "-");
                println!("Using key for {:?}{:?}", f.key, safe_name);
                let obj = bucket.get_object(f.key.clone()).await?;
                // Should upload/update the file
                //
                let maybe_uploaded = get_by_filename(&uploaded_files_info, &safe_name);
                match maybe_uploaded {
                    None => {
                        let b = obj.to_vec().to_owned();
                        if b.len() > 0 {
                            send_as_file(safe_name, b).await?
                        }
                    }
                    Some(owui_f) => match owui_f.meta.content_type {
                        None => delete_file(owui_f).await?,
                        _ => {
                            if owui_f.updated_at < f_modified {
                                println!(
                                    "Src file is newer than owui: {} owui: {}",
                                    f_modified, owui_f.updated_at
                                );

                                update_file(owui_f, obj.to_string()?.as_str()).await?
                            } else {
                                println!(
                                    "owui file is newer than src: {} owui: {}",
                                    f_modified, owui_f.updated_at
                                )
                            }
                        }
                    },
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
    let mut file_part_headers = reqwest::header::HeaderMap::new();
    file_part_headers.insert(
        reqwest::header::CONTENT_TYPE,
        "application/octet-stream".parse()?,
    );
    let file_part = multipart::Part::bytes(data)
        .file_name(name.clone())
        .headers(file_part_headers);
    let form = form.part("file", file_part);
    let client = reqwest::Client::new();
    let owui_url = env::var("OPENWEBUI_URL").expect("env OPENWEBUI_URL must be set");
    let owui_token =
        env::var("OPENWEBUI_BEARER_TOKEN").expect("env OPENWEBUI_BEARER_TOKEN must be set");
    let owui_kid =
        env::var("OPENWEBUI_KNOWLEDGE_ID").expect("env OPENWEBUI_KNOWLEDGE_ID must be set");
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

#[derive(Deserialize, Serialize, Debug)]
struct FileContentUpdate {
    content: String,
}

async fn delete_file(file: &FileResponse) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let owui_url = env::var("OPENWEBUI_URL").expect("env OPENWEBUI_URL must be set");
    let owui_token =
        env::var("OPENWEBUI_BEARER_TOKEN").expect("env OPENWEBUI_BEARER_TOKEN must be set");
    let id = file.id.clone();
    let owui_kid =
        env::var("OPENWEBUI_KNOWLEDGE_ID").expect("env OPENWEBUI_KNOWLEDGE_ID must be set");
    let file_delete_url = format!("{owui_url}/api/v1/files/{id}");
    let bearer = format!("Bearer {owui_token}");

    println!(
        "Deleting file: {:?} with meta.content_type: {:?} ",
        file.filename, file.meta.content_type
    );
    let resp = client
        .delete(file_delete_url)
        .header("Authorization", bearer.clone())
        .header("Content-Type", "application/json")
        .send()
        .await?;

    let maybe_err = resp.error_for_status_ref().map(|_| ());
    if resp.status() != 200 {
        let t = resp.text().await?;
        println!("error deleting file: {t}");
    }
    maybe_err?;
    Ok(())
}

async fn update_file(file: &FileResponse, data: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let owui_url = env::var("OPENWEBUI_URL").expect("env OPENWEBUI_URL must be set");
    let owui_token =
        env::var("OPENWEBUI_BEARER_TOKEN").expect("env OPENWEBUI_BEARER_TOKEN must be set");
    let id = file.id.clone();
    let owui_kid =
        env::var("OPENWEBUI_KNOWLEDGE_ID").expect("env OPENWEBUI_KNOWLEDGE_ID must be set");
    let file_update_url = format!("{owui_url}/api/v1/files/{id}/data/content/update");
    let knowledge_url_update = format!("{owui_url}/api/v1/knowledge/{owui_kid}/file/update");
    let knowledge_url_remove = format!("{owui_url}/api/v1/knowledge/{owui_kid}/file/remove");
    let knowledge_url_add = format!("{owui_url}/api/v1/knowledge/{owui_kid}/file/add");
    let bearer = format!("Bearer {owui_token}");

    let file_update = FileContentUpdate {
        content: data.to_string(),
    };

    println!(
        "Updating file: {:?} to URL {:?} with meta.content_type: {:?} ",
        file.filename, file_update_url, file.meta.content_type
    );
    let resp = client
        .post(file_update_url)
        .header("Authorization", bearer.clone())
        .header("Content-Type", "application/json")
        .json(&file_update)
        .send()
        .await?;

    let maybe_err = resp.error_for_status_ref().map(|_| ());
    if resp.status() != 200 {
        let t = resp.text().await?;
        println!("error updating file: {t}");
    }

    let add = AddFileToKnowledgeBase { file_id: id };
    println!("Updating knowledgebase request: {:?}", add);
    // Running into an error in owui:
    //      Expected metadata value to be a str, int, float or bool, got None which is a NoneType
    //      Traceback (most recent call last):
    //      File "/app/backend/open_webui/apps/retrieval/main.py", line 868, in save_docs_to_vector_db
    //      VECTOR_DB_CLIENT.insert(
    //
    // using this code:
    let resp = client
        .post(knowledge_url_update)
        .header("Authorization", &bearer)
        .header("Content-Type", "application/json")
        .json(&add)
        .send()
        .await?;
    if resp.status() != 200 {
        let t = resp.text().await?;
        println!("error trying to update: {t}");
    } else {
        return Ok(());
    }

    println!("Going to try and add the item");
    //
    // Going to delete then add it back to the knowledgebase
    //
    //println!("Removing knowledgebase item; request: {:?}", knowledge_url_remove);
    //let resp = client
    //    .post(knowledge_url_remove)
    //    .header("Authorization", bearer.clone())
    //    .header("Content-Type", "application/json")
    //    .json(&add)
    //    .send()
    //    .await?;
    //let maybe_err = resp.error_for_status_ref().map(|_| ());
    //if resp.status() != 200 {
    //    let t = resp.text().await?;
    //    println!("error: {t}");
    //    maybe_err?;
    //}

    println!(
        "Adding item knowledgebase; request: {:?}",
        knowledge_url_add
    );
    let resp = client
        .post(knowledge_url_add)
        .header("Authorization", &bearer)
        .header("Content-Type", "application/json")
        .json(&add)
        .send()
        .await?;
    //let maybe_err = resp.error_for_status_ref().map(|_| ());
    if resp.status() != 200 {
        let t = resp.text().await?;
        println!("error: {t}");
    }
    Ok(())
}

use chrono::serde::ts_seconds;
#[derive(Deserialize, Serialize, Debug)]
struct FileResponse {
    id: String,
    filename: String,
    #[serde(with = "ts_seconds")]
    created_at: chrono::DateTime<chrono::Utc>,
    #[serde(with = "ts_seconds")]
    updated_at: chrono::DateTime<chrono::Utc>,
    meta: FileMeta,
}

#[derive(Deserialize, Serialize, Debug)]
struct FileMeta {
    content_type: Option<String>,
}

fn get_by_filename<'a>(files: &'a Vec<FileResponse>, name: &'a str) -> Option<&'a FileResponse> {
    files.into_iter().find(|&f| f.filename == name)
}

async fn get_uploaded_files_info() -> anyhow::Result<Vec<FileResponse>> {
    let client = reqwest::Client::new();
    let owui_url = env::var("OPENWEBUI_URL").expect("env OPENWEBUI_URL must be set");
    let owui_token =
        env::var("OPENWEBUI_BEARER_TOKEN").expect("env OPENWEBUI_BEARER_TOKEN must be set");
    let bearer = format!("Bearer {owui_token}");
    let files_url = format!("{owui_url}/api/v1/files/");
    let resp = client
        .get(files_url)
        .header("Authorization", bearer.clone())
        .send()
        .await?;
    let files: Vec<FileResponse> = resp.json().await?;
    Ok(files)
}
