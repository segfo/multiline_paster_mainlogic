$dest_dirs=@("D:\Users\segfo\Downloads\plugins","D:\Users\segfo\Downloads\full_package\multiline_paster_plugins","$Env:userprofile\.cargo\bin\multiline_paster_plugins")
$src=".\target\debug\main_logic.dll"

foreach ($dest in $dest_dirs){
    copy -Force "$src" "$dest"
}
