import http from 'k6/http';
import { check } from 'k6';
import { SharedArray } from 'k6/data';

// Predefined array of identity commitments
const identityCommitments = new SharedArray('identityCommitments', function () {
  return [
    "0x04c078aecf167bd3c9d34bbff2f50ff53e15a48977fb55cf7ef4189da18fb005",
    "0x13314c6519c8fb5b497ed8bdc358febb23324826b8076cb670eb5f32e7633b1a",
    "0x01d842d4d9eb41b687e3e3dac314190e5c84b479b3caaf547120f1c1b0829068",
    "0x14c241bb7aaa404dedebb39ce8cc71261d73b949fcf4bbf4c9b0bbbd7c01f155",
    "0x2b777c9b005e963cd45c8c41515e6640f107f0b48a986d8a4cbecd6ee822adfd",
    "0x01cdfe6f7682c8904ac06d518438da3193af0d5b70d752bb96c86811ed5e3801",
    "0x10391422e465a19bac708ef17a5ea20e44b1dc36e7b140361fc94a9b9ef7871d",
    "0x111cafedafda852c408e81dbc05789d20dcaf8d189a2a13d0aaf99005290cd2f",
    "0x22e48d25826b4df7e377bccd822c23fad50216638f22fd6b66fe081819077b80",
    "0x05bbcd5c60aeac4fcf5e4a9392822af5c68a47d10b6d71383092d4c2767b4a22",
    "0x20bf5877807afe4dd46117f546c6e4abaa570e87d822de79c097003182a225bd",
    "0x1f38e5bb5a2c9a24394c731bd0019059afda90c3630a42080e8ae1f00cc990c0",
    "0x026c1e9f7fbfdfd018b436181924d825d91d1f0ee728738ec5210935ac880cf7",
    "0x22dd9698092ac90cebacc95bd8962e2196d66c74c8f7b4f13c3e2b8f23b0365d",
    "0x1133046dcab2fc0262dee6feffb155d53e6ea8e33a7261720a02c520545f2f09",
    "0x0f4a64afe240db3024fa5ffbf606634e5e0b3401f59d1031a891389c5c6a3b48",
    "0x1caf326a84cd2674df382ba42b5e052bbe54170b175427148599f82f9a82a91d",
    "0x086ccfdf7b66f818fc8687a4fbde2dd128f7371785920e9cb21bb219672cb7c8",
    "0x0e4f7120be3607cd32ad28bb3f0cfc6beafc9c4d2f4c5c6c9194e650ccf354d7",
    "0x2e535e66c507acbcc7887256d8bd6c83b7ab04920a260137e9aede4e464a9e52",
    "0x0516a2eceb9b3367d0ccff9ec0ddd972aaf6246aa2d5cf645b0208bb6661c821",
    "0x2da13664e17a1473bc1e357333ff793bea296b0c3a51f3bb90542f2d3f8cfc59",
    "0x007b7316bca2e0da5ea0932fcba73a496d9a4dbb5d46968ce61fbe7459a49097",
    "0x0feb17a3853ba4a3fdb785005f154a33fa27e399d8e5a105e08c0c85a845d941",
    "0x1269c3b3d8c49dfad2ee856ed015ec52ef8d271fc4dbb02a20a74e2048fcafe2",
    "0x1982c3c14074bf73eaee435bf81bee2ef0810ec1abce354c5ec29cf05a6ec009",
    "0x0e02fd7a0a82a486f5b1f7cdd4413c1f7c355b351cd7038746ceacaf6f62a658",
    "0x169084d7f7130d9bbdf3711b4c3ff5b009173b172cd31b25bab56d3137466ba2",
    "0x28bd90ba81c14be70444757f8e84e685062eb94ff76a13a6c3df9701b3ade800",
    "0x0f1e954b29ac22e9f32de017ca32f233bb72e69b813feeedf336bce7706dc3b4",
    "0x21701196735480bd458b9cc6d92d46a739f188877a1e223f52afbd8ed6a9955b",
    "0x067916fd2e2d38749cf2c03116254d4e489d82e3af0f006ab67a03b7df7dfb39",
    "0x02d10d932077d98dcdb5b148ca8ba9f58abd653e30e1bad37d70ff58abae51f6",
    "0x223273b507af47b440223beadbfde1fd920dce9359aaee0dc12d225b4f580483",
    "0x084e5063825dfef28e37d07a9d1626e3b1e05809667c8b0b28a86b5b33dc27cf",
    "0x035d7c83b08dcb2629e69b37ca908722a441990bac3f408be293a588b74eb341",
    "0x304415e4b83411f4ddb66f7dc28892ba6f0ee8d4c26d097b6ea226a2c061f24e",
    "0x018e9c28fd8cf53c60e098fbfacdbbfc5bec6fa494d5a8373fb7ee6f2fb0977a",
    "0x1a1a8f91c21845ac1f8788dbc6a9408952b429c99a91bdb9d79f46e9f7984bda",
    "0x18f78c26a239a9aed766efde1eb8d65be2393866bd3738d0c5eb78b6a9d27824",
    "0x27ce275d303117e714d2ff304a4af120b64da1e66c5d8f1838cc17bae0ca82f3",
    "0x0f3986458c66460ae9c6573ac41d5d67ce0724dd52ffb35cce4ac0a72aadb926",
    "0x231a69e1b9ba13aeb723d7fa7c3864147a491490d1084f1077ec7cf12719ebf6",
    "0x0a7e79bf0ac35a851b66fd760e6f9804c1fadbbd7c1d07069673c120bdd174ba",
    "0x0b4c39d52c0ed43b4be16d032b8ca9dfe19bd9e4d82ba3f1c4cb4d75805f27b8",
    "0x0030e1e5da225352ba624e1e672f1bbb0fca6c467ed3459dfba40ad5f49b0184",
    "0x2b97510c4725979553b1b16635867c2b3f5990d824ad43bba3261828136facd5"
  ];
});




// Initialize the endpoint
const baseEndpoint = __ENV.TREE_AVAILABILITY_SERVICE_ENDPOINT;
if (!baseEndpoint) {
  throw new Error('Base endpoint not specified. Use -e TREE_AVAILABILITY_SERVICE_ENDPOINT=your_endpoint');
}
const endpoint = `${baseEndpoint}/inclusionProof`;

export default function () {
  const identityCommitment = identityCommitments[Math.floor(Math.random() * identityCommitments.length)];

  // Construct the request body
  const body = JSON.stringify({
    identityCommitment: identityCommitment,
  });

  // Send the request
  const res = http.post(endpoint, body, {
    headers: { 'Content-Type': 'application/json' },
  });

  // Check the response status
  check(res, { 'status was 200': (r) => r.status === 200 });

  // Check if proof value is present
  const responseData = JSON.parse(res.body);
  check(responseData, { 'proof is present': (data) => data.proof !== undefined && data.proof !== null });
}
