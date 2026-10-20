# miliastra/collab
A Windows app that aims to solve some flaws of the Shared Creation system in Miliastra Wonderland. Facilitates slice-locking, automatic syncing, and version history, among other things.

Uses either OneDrive or a local folder for sharing stage files. Note that if you install Google Drive / OneDrive / Dropbox on your machine, you can point to it as local folder and share it that way.

Important: there is a small coordination server running that this app connects to. All data sent through this is encrypted and non-identifyable. The server cannot read what is being sent, and only a tiny bit of meta data is sent to coordinate slice locking. Stage files are not sent through this.

## How to use

Download `miliastra-collab.exe` from the latest GitHub release and open it.

- **Create:** enter a project name and your name, choose shared-folder or OneDrive storage.
- **Join:** have someone invite you to their team, then click **Open miliastra/collab** on the shared 

## Coordinator address

Normal builds use the hosted coordinator at `https://collabs.miliastra.dev` 
The address is stored in `%LOCALAPPDATA%/dev.miliastra.collab/coordination.json`:

```json
{ "serviceUrl": "https://collabs.miliastra.dev" }
```

Advanced users can change this to their own coordinator service

## License

miliastra/collab is free software under the GNU General Public License, version 3 or later. See [LICENSE](LICENSE). The coordinator is a separate project under the GNU Affero General Public License, version 3 or later.

The license covers the code, not the miliastra/collab name and logo. If you distribute a modified version, give it its own name and logo. This is a community project and isn't affiliated with HoYoverse.
