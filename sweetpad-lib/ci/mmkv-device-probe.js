// An offline probe for the isolated physical-device fixture. The existing
// example remains visible beneath the results. No Metro server is required.
import React from 'react';
import {AppRegistry, Text, View} from 'react-native';
import {createMMKV} from 'react-native-mmkv';
import App from './src/App';
import {name} from './app.json';

const checks = {};
let phase = 0;
let error = null;
try {
  const storage = createMMKV({id: 'dev.sweetpad.parity'});
  phase = (storage.getNumber('launches') ?? 0) + 1;
  checks.persistence = phase === 1 || storage.getString('persistent') === 'Linux/Apple 🦀';
  storage.set('persistent', 'Linux/Apple 🦀');
  storage.set('number', -42.5);
  checks.number = storage.getNumber('number') === -42.5;
  storage.set('boolean', true);
  checks.boolean = storage.getBoolean('boolean') === true;
  const bytes = new Uint8Array([0, 1, 127, 255]);
  storage.set('buffer', bytes.buffer);
  checks.buffer = Array.from(new Uint8Array(storage.getBuffer('buffer'))).join() === bytes.join();
  storage.set('remove', 'temporary');
  checks.keys = storage.getAllKeys().includes('remove');
  storage.remove('remove');
  checks.remove = !storage.contains('remove') && storage.getString('remove') === undefined;
  checks.string = storage.getString('persistent') === 'Linux/Apple 🦀';
  storage.set('launches', phase);
  storage.set('sweetpadDeviceReport', 'SWEETPAD_REPORT_BEGIN' + JSON.stringify({phase, checks, error}) + 'SWEETPAD_REPORT_END');
} catch (e) {
  error = String(e.stack ?? e);
}

function DeviceProbe() {
  const passed = Object.values(checks).filter(Boolean).length;
  return <View style={{flex: 1, backgroundColor: 'white', paddingTop: 65}}>
    <Text style={{fontSize: 20, color: 'black', padding: 12}}>
      Linux MMKV: {passed}/7 checks; launch {phase}{error ? '\n' + error : ''}
    </Text>
    <App />
  </View>;
}
AppRegistry.registerComponent(name, () => DeviceProbe);
