// Copyright 2018-2019 Parity Technologies (UK) Ltd.
// This file is part of ink!.
//
// ink! is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// ink! is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with ink!.  If not, see <http://www.gnu.org/licenses/>.

use std::{
    env, fs,
    io::{Cursor, Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

use anyhow::Result;
use heck::CamelCase as _;

pub(crate) fn execute_new(
    name: &str,
    dir: Option<&PathBuf>,
) -> Result<String> {
    if name.contains('-') {
        anyhow::bail!("Contract names cannot contain hyphens");
    }

    let out_dir = dir.unwrap_or(&env::current_dir()?).join(name);
    if out_dir.join("Cargo.toml").exists() {
        anyhow::bail!("A Cargo package already exists in {}", name);
    }
    if !out_dir.exists() {
        fs::create_dir(&out_dir)?;
    }

    let template = include_bytes!(concat!(env!("OUT_DIR"), "/template.zip"));
    let mut cursor = Cursor::new(Vec::new());
    cursor.write_all(template)?;
    cursor.seek(SeekFrom::Start(0))?;

    let mut archive = zip::ZipArchive::new(cursor)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;

        // replace template placeholders
        let contents = contents.replace("{{name}}", name);
        let contents = contents.replace("{{camel_name}}", &name.to_camel_case());

        let outpath = out_dir.join(file.sanitized_name());

        if (&*file.name()).ends_with('/') {
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    fs::create_dir_all(&p)?;
                }
            }
            let mut outfile = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(outpath.clone())
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::AlreadyExists {
                        anyhow::anyhow!(
                            "New contract file {} already exists",
                            file.sanitized_name().display()
                        )
                    } else {
                        anyhow::anyhow!(e)
                    }
                })?;

            outfile.write_all(contents.as_bytes())?;
        }

        // Get and set permissions
        #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                if let Some(mode) = file.unix_mode() {
                    fs::set_permissions(&outpath, fs::Permissions::from_mode(mode))?;
                }
            }
    }

    Ok(format!("Created contract {}", name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cmd::{execute_new, tests::with_tmp_dir},
    };

    #[test]
    fn rejects_hyphenated_name() {
        with_tmp_dir(|path| {
            let result = execute_new(
                "rejects-hyphenated-name",
                Some(path),
            );
            assert_eq!(
                format!("{:?}", result),
                r#"Err(Contract names cannot contain hyphens)"#
            )
        });
    }

    #[test]
    fn contract_cargo_project_already_exists() {
        with_tmp_dir(|path| {
            let name = "test_contract_cargo_project_already_exists";
            let _ = execute_new(name, Some(path));
            let result = execute_new(name, Some(path));
            assert_eq!(
                format!("{:?}", result),
                r#"Err(A Cargo package already exists in test_contract_cargo_project_already_exists)"#
            )
        });
    }

    #[test]
    fn dont_overwrite_existing_files_not_in_cargo_project() {
        with_tmp_dir(|path| {
            let name = "dont_overwrite_existing_files";
            let dir = path.join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::File::create(dir.join(".gitignore")).unwrap();
            let result = execute_new(name, Some(path));
            assert_eq!(
                format!("{:?}", result),
                r#"Err(New contract file .gitignore already exists)"#
            )
        });
    }
}
