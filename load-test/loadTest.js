import http from 'k6/http';
import { check } from 'k6';
import { SharedArray } from 'k6/data';

// Predefined array of identity commitments
const identityCommitments = new SharedArray('identityCommitments', function () {
  return [
    "0x006d993de019fe646fab3467b7c1686f97ae817b649a79285143a87c3b5a1852",
    "0x1fd25aeb2106133ac341f6f07a1bc40ec8d27b9fe1a34fe8f33980373d59552f", 
    "0x1f9892d41f60f9eed5bd1cc8c0d4da2d5074ba60c65ce12ee4310ac086fdfaaf",
    "0x1fa6d21e0197f078536289cdc3c547caa5f2b6981c901abf0eacf932e699d4de",
    "0x28ece58de7c7f11ade349bacdfb94b1d363747776a964c3e8a39b8f1942c8440",
    "0x0de41f71c825759a7cfdc8c9bd37cc2c54b48e02e20a2caff6d3eeccb67089c6",
    "0x07d5c1cb03d265a2285525cf582445de73356373758faf7cf30d674710980949",
    "0x1fbec6c35295637c220976666b857e9cc665c35f40300ed9008ed4df57cae755",
    "0x102420225de40328691baab5f923178cf71d827a6d81db31355898f5e3ce1166",
    "0x03dd150095f602e27fcafc8c99929c730fb4b40811ac9570b31249b47bf25cde",
    "0x03b52e33c86815be00f26345ece2d3d9ad58f3cc059bfac6dcd2ba13f4aa63df",
    "0x1fc3d1a7b0e984c22b7f5b57c2a8f9cfda7aa02f7e82d2a93c5dbe3d17a5f768",
    "0x2d4c0588bdaac7c3d36b206a30d576cca2dd700a0e4c9438b4c4d0cea3aa9a71",
    "0x11aa25937ae7264f2862813ede24a7722c22743055cf749a6158d0d2693790ac",
    "0x27f0c1497f5a5d422b7909b4a1bb689bfd151fcd8034ed335d2e8f8650459058",
    "0x2797a92de8606100493cd43460eb49072c87a2fa135fa347655509385a457d8a",
    "0x1ffb97031f2f42a2d51031d27f246ce521c5dc524065de821f90a5f47b94753b",
    "0x030b17ba34cb8a4958eb78cdf22f3a37f1d4b6c1730696ed41359ef652945e04",
    "0x0cfdd4ea94942e75b1611d4486ff1229f9453bbf7dcae54f516f62df0ef1ee27",
    "0x148f17e07110e36071cbf5323d6ac6e1cbc1e2efd67e6bbb3ed8a200ce73d2de",
    "0x1a451896169025efef06560d1d5abe7b0a3618362abf1dd3e7f3410f8c91315e",
    "0x214696d523f472b9dc81ef0c81e24e56c265281266c681ae07a1760f41f38116",
    "0x1d68bc984ca5e9ccc596039540aecd4655af7459fc234d41661bd8f107ca22ae",
    "0x12597fb061ecafb8b30bac92444ee471eba703740419b14d94796212f7910309",
    "0x3041ae03b1fbb416fac2c423b72ddbfbfb24495876b203b9a5853f0fd7387910",
    "0x0d3ebe1716d117128feaba1e0c36462e0d4d1ba2f7bdd7e2086ba4420ce4bafa"
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
