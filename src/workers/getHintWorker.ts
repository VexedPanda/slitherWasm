import HintWorker from './hintWorker.ts?worker';

import { TinyProperty } from 'scenerystack/axon';

let hintWorker: Worker | null = null;

export const hintWorkerLoadedProperty = new TinyProperty(false);

export const getHintWorker = (): Worker => {
  if (!hintWorker) {
    hintWorker = new HintWorker();

    hintWorker?.addEventListener('message', (event) => {
      if (event.data.type === 'hint-worker-loaded') {
        hintWorkerLoadedProperty.value = true;
      }
    });
  }

  return hintWorker!;
};

// (Created on the first hint request rather than on startup: its pattern data takes memory that phones need, e.g. for
// large boards)
