[Setup]
AppName=ZONA
AppVersion=0.1.0
DefaultDirName={pf}\ZONA
DefaultGroupName=ZONA
OutputDir=..\dist
OutputBaseFilename=ZONA-Setup
Compression=lzma
SolidCompression=yes

[Files]
Source: "..\launcher\dist\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs
Source: "..\target\release\zn-client.exe"; DestDir: "{app}\game"; Flags: ignoreversion
Source: "..\target\release\zn-server.exe"; DestDir: "{app}\game"; Flags: ignoreversion

[Icons]
Name: "{group}\ZONA Launcher"; Filename: "{app}\zona-launcher.exe"
Name: "{commondesktop}\ZONA Launcher"; Filename: "{app}\zona-launcher.exe"

[Run]
Filename: "{app}\zona-launcher.exe"; Description: "Launch ZONA"; Flags: nowait postinstall skipifsilent
