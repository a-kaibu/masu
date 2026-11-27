// One broker handles the page's requests and the ports of its game threads.
import { masuFetchWorker } from './fetch-bridge.js';

self.onmessage = (event) => {
  const port = event.data.port;
  if (port) port.onmessage = masuFetchWorker;
  else masuFetchWorker(event);
};
