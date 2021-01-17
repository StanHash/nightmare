
use std::collections::BTreeMap;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::fs::File;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error
{
    #[error("At {filename}:{line}: {source}")]
    Located
    {
        filename: PathBuf,
        line: usize,
        source: Box<Error>,
    },

    #[error("Invalid module version (it can only be \"1\")")]
    InvalidModuleVersion,

    #[error("Invalid component kind")]
    InvalidComponentKind,

    #[error("Malformed charset file")]
    InvalidCharset,

    #[error("Too many component entries")]
    TooManyComponentEntries,

    #[error("Unexpected end of module file")]
    UnexpectedEof,

    #[error("IO error: {source}")]
    Io
    {
        #[from]
        source: io::Error,
    },

    #[error("Int parse error: {source}")]
    ParseInt
    {
        #[from]
        source: std::num::ParseIntError,
    },
}

#[derive(Debug, PartialEq)]
pub enum NumberFormat
{
    Hex,
    Dec,
    DecSigned,
}

#[derive(Debug, PartialEq)]
pub enum ComponentKind
{
    Text,
    HexArray,
    Number(NumberFormat),
    Dropbox(NumberFormat, Vec<(u32, String)>),
}

#[derive(Debug, PartialEq)]
pub struct Component
{
    pub description: String,
    pub offset: u32,
    pub length: u32,
    pub kind: ComponentKind,
}

#[derive(Default, Debug, PartialEq)]
pub struct Module
{
    pub description: String,
    pub root_offset: u32,
    pub entry_count: u32,
    pub entry_length: u32,
    pub entry_names: Option<Vec<String>>,
    pub charset: Option<BTreeMap<u8, char>>,
    pub components: Vec<Component>,
}

pub fn from_file<P>(filename: P) -> Result<Module, Error>
    where P: AsRef<Path>
{
    enum ReadState
    {
        ReadVersion,
        ReadDescription,
        ReadRootOffset,
        ReadEntryCount,
        ReadEntryLength,
        ReadEntryNames,
        ReadCharset,
        ReadNextComponentDescription,
        ReadComponentOffset,
        ReadComponentLength,
        ReadComponentKind,
        ReadComponentDropboxEntriesAndEnd,
    }

    let mut result = Module::default();
    let mut read_state = ReadState::ReadVersion;

    let parent_dir = filename.as_ref().parent().unwrap_or_else(|| Path::new(""));

    let mut current_component_desc: Option<String> = None;
    let mut current_component_offset = 0u32;
    let mut current_component_length = 0u32;
    let mut current_component_kind_str: Option<String> = None;

    for (i, line) in read_lines(&filename)?.enumerate()
    {
        let line_str = line?;
        let line = line_str.trim();

        // skip empty lines
        if line.is_empty() { continue }

        // skip comments
        if line.starts_with('#') { continue }

        let located_err = |err: Error| Error::Located { filename: filename.as_ref().into(), line: i + 1, source: Box::new(err) };

        match read_state
        {
            ReadState::ReadVersion =>
            {
                match line
                {
                    "1" => {},
                    _ => { return Err(Error::InvalidModuleVersion) }
                }

                read_state = ReadState::ReadDescription;
            }

            ReadState::ReadDescription =>
            {
                result.description = line_str;
                read_state = ReadState::ReadRootOffset;
            }

            ReadState::ReadRootOffset =>
            {
                result.root_offset = parse_int(line).map_err(located_err)?;
                read_state = ReadState::ReadEntryCount;
            }

            ReadState::ReadEntryCount =>
            {
                result.entry_count = parse_int(line).map_err(located_err)?;
                read_state = ReadState::ReadEntryLength;
            }

            ReadState::ReadEntryLength =>
            {
                result.entry_length = parse_int(line).map_err(located_err)?;
                read_state = ReadState::ReadEntryNames;
            }

            ReadState::ReadEntryNames =>
            {
                result.entry_names = read_module_entries(get_full_filename(parent_dir, line))?;
                read_state = ReadState::ReadCharset;
            }

            ReadState::ReadCharset =>
            {
                match get_full_filename(parent_dir, line)
                {
                    Some(filename) => { result.charset = Some(read_charset(filename)?); }
                    None => {}
                }

                read_state = ReadState::ReadNextComponentDescription;
            }

            ReadState::ReadNextComponentDescription =>
            {
                current_component_desc = Some(line_str);
                read_state = ReadState::ReadComponentOffset;
            }

            ReadState::ReadComponentOffset =>
            {
                current_component_offset = parse_int(line).map_err(located_err)?;
                read_state = ReadState::ReadComponentLength;
            }

            ReadState::ReadComponentLength =>
            {
                current_component_length = parse_int(line).map_err(located_err)?;
                read_state = ReadState::ReadComponentKind;
            }

            ReadState::ReadComponentKind =>
            {
                current_component_kind_str = Some(line_str);
                read_state = ReadState::ReadComponentDropboxEntriesAndEnd;
            }

            ReadState::ReadComponentDropboxEntriesAndEnd =>
            {
                result.components.push(build_component(
                    current_component_desc.take().unwrap(),
                    current_component_offset,
                    current_component_length,
                    &current_component_kind_str.take().unwrap(),
                    get_full_filename(parent_dir, line))?);

                read_state = ReadState::ReadNextComponentDescription;
            }
        }
    }

    match read_state
    {
        ReadState::ReadNextComponentDescription => Ok(result),
        _ => Err(Error::UnexpectedEof),
    }
}

fn build_component<P>(description: String, offset: u32, length: u32, kind_str: &str, dropbox_entry_file: Option<P>) -> Result<Component, Error>
    where P: AsRef<Path>
{
    let kind = match kind_str.trim()
    {
        "TEXT" => ComponentKind::Text,
        "HEXA" => ComponentKind::HexArray,
        "NEHU" => ComponentKind::Number(NumberFormat::Hex),
        "NEDU" => ComponentKind::Number(NumberFormat::Dec),
        "NEDS" => ComponentKind::Number(NumberFormat::DecSigned),
        "NDHU" => ComponentKind::Dropbox(NumberFormat::Hex, read_component_dropbox_entries(dropbox_entry_file)?),
        "NDDU" => ComponentKind::Dropbox(NumberFormat::Dec, read_component_dropbox_entries(dropbox_entry_file)?),

        _ => return Err(Error::InvalidComponentKind),
    };

    Ok(Component
    {
        description: description,
        offset: offset,
        length: length,
        kind: kind,
    })
}

fn get_full_filename<P>(parent_dir: P, filename: &str) -> Option<PathBuf>
    where P: AsRef<Path>
{
    match filename
    {
        "NULL" => None,
        _ => Some(parent_dir.as_ref().join(filename))
    }
}

fn read_module_entries<P>(filename: Option<P>) -> Result<Option<Vec<String>>, Error>
    where P: AsRef<Path>
{
    if let Some(filename) = filename
    {
        let mut result: Vec<String> = Vec::new();

        for line in read_lines(filename)?
        {
            result.push(line?);
        }

        return Ok(Some(result));
    }

    Ok(None)
}

fn read_charset<P>(filename: P) -> Result<BTreeMap<u8, char>, Error>
    where P: AsRef<Path>
{
    let mut result: BTreeMap<u8, char> = BTreeMap::new();

    for (i, line) in read_lines(&filename)?.enumerate()
    {
        let line = line?;
        let line = line.trim();

        let located_err = |err: Error| Error::Located { filename: filename.as_ref().into(), line: i + 1, source: Box::new(err) };

        let split: Vec<&str> = line.split('=').map(|s| s.trim()).collect();

        if split.len() != 2
        {
            return Err(located_err(Error::InvalidCharset));
        }

        let number = u32::from_str_radix(split[0], 16).map_err(|err| located_err(err.into()))?;
        let character = split[1].chars().next().unwrap_or('\x00');

        result.insert(number as u8, character);
    }

    Ok(result)
}

fn read_component_dropbox_entries<P>(filename: Option<P>) -> Result<Vec<(u32, String)>, Error>
    where P: AsRef<Path>
{
    if let Some(filename) = filename
    {
        let mut result: Vec<(u32, String)> = Vec::new();

        let mut is_first_line = true;
        let mut entries_left = 0;

        for (i, line) in read_lines(&filename)?.enumerate()
        {
            let line = line?;
            let line = line.trim();

            let located_err = |err: Error| Error::Located { filename: filename.as_ref().into(), line: i + 1, source: Box::new(err) };

            if is_first_line
            {
                entries_left = parse_int(line).map_err(located_err)?;
                is_first_line = false;
            }
            else
            {
                if entries_left == 0
                {
                    return Err(Error::TooManyComponentEntries);
                }

                let split: Vec<&str> = line.splitn(2, ' ').collect();

                let num = parse_int(split[0]).map_err(located_err)?;
                result.push((num, split[1].into()));

                entries_left -= 1;
            }
        }

        return Ok(result);
    }

    Ok(Vec::new())
}

fn read_lines<P>(filename: P) -> io::Result<io::Lines<io::BufReader<File>>>
    where P: AsRef<Path>
{
    let file = File::open(filename)?;
    Ok(io::BufReader::new(file).lines())
}

fn parse_int(input: &str) -> Result<u32, Error>
{
    if input.starts_with('0')
    {
        let input = &input[1..];

        if input.is_empty()
        {
            return Ok(0);
        }

        if input.starts_with('x')
        {
            Ok(u32::from_str_radix(&input[1..], 16)?)
        }
        else
        {
            Ok(u32::from_str_radix(input, 8)?)
        }
    }
    else
    {
        Ok(u32::from_str_radix(input, 10)?)
    }
}

#[cfg(test)]
mod tests
{
    use super::*;

    #[test]
    fn read_module()
    {
        let _ = from_file("dat/SpellAssoc.nmm").unwrap();
    }
}
