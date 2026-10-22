'use strict';
const { contextBridge, ipcRenderer, webUtils } = require('electron');

contextBridge.exposeInMainWorld('api', {
  loadLibrary: () => ipcRenderer.invoke('library:load'),
  saveLibrary: (data) => ipcRenderer.invoke('library:save', data),
  readFile: (path) => ipcRenderer.invoke('file:read', path),
  filesExist: (paths) => ipcRenderer.invoke('files:exist', paths),
  expandPaths: (paths) => ipcRenderer.invoke('paths:expand', paths),
  pickFiles: () => ipcRenderer.invoke('dialog:pick-files'),
  readSoundpadList: (pick) => ipcRenderer.invoke('soundpad:read', pick),
  setHotkeys: (list) => ipcRenderer.invoke('hotkeys:set', list),
  onHotkey: (fn) => ipcRenderer.on('hotkey', (_e, action) => fn(action)),
  pathForFile: (file) => webUtils.getPathForFile(file),
  openVbCable: () => ipcRenderer.invoke('open:vbcable'),
  sendMicPcm: (pcm) => ipcRenderer.send('mic:pcm', pcm),
  micFilter: (action) => ipcRenderer.invoke('mic:filter', action),
  onMicState: (fn) => ipcRenderer.on('mic:state', (_e, s) => fn(s)),
});
