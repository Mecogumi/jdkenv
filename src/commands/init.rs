use anyhow::Result;

pub fn run() -> Result<()> {
    print!(
        r#"function jdkenv {{
    $exe = "$env:USERPROFILE\.jdkenv\bin\jdkenv.exe"
    if ($args.Count -ge 1 -and $args[0] -eq 'set') {{
        $out = & $exe @args
        if ($LASTEXITCODE -eq 0 -and $out) {{ $out | Invoke-Expression }} else {{ $out }}
    }} else {{
        & $exe @args
    }}
}}
"#
    );
    Ok(())
}
