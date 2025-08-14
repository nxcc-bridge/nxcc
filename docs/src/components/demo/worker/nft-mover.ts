// NXCC Worker for moving NFTs between chains
import { worker } from "@nxcc/sdk";

export default worker({
  async launch(eventPayload, { userdata }) {
    console.log("NFT mover worker launched with event:", eventPayload);

    // Handle blockchain events that trigger NFT moves
    if (eventPayload?.event === "NFTMoved") {
      const { tokenId, fromChain, toChain, ownerAddress } = eventPayload.data;
      console.log(
        `Processing NFT move: Token ${tokenId} from ${fromChain} to ${toChain}`,
      );

      // Process the cross-chain NFT move
      const result = await moveNFT(tokenId, fromChain, toChain, ownerAddress);

      return new Response(
        JSON.stringify({
          status: "processed",
          result,
        }),
        {
          headers: { "Content-Type": "application/json" },
        },
      );
    }

    return new Response("Event handled");
  },

  async fetch(request, { userdata }) {
    const url = new URL(request.url);

    if (url.pathname === "/move-nft" && request.method === "POST") {
      try {
        const { tokenId, fromChain, toChain, ownerAddress } =
          await request.json();

        // Simulate NFT move logic
        // In a real implementation, this would:
        // 1. Verify the NFT burn event on the source chain
        // 2. Mint a new NFT on the target chain
        // 3. Maintain cross-chain state consistency

        const result = await moveNFT(tokenId, fromChain, toChain, ownerAddress);

        return new Response(JSON.stringify(result), {
          headers: { "Content-Type": "application/json" },
        });
      } catch (error) {
        return new Response(
          JSON.stringify({ error: (error as Error).message }),
          {
            status: 500,
            headers: { "Content-Type": "application/json" },
          },
        );
      }
    }

    if (url.pathname === "/status" && request.method === "GET") {
      return new Response(
        JSON.stringify({
          status: "active",
          supportedChains: ["ethereum", "polygon", "arbitrum", "optimism"],
        }),
        {
          headers: { "Content-Type": "application/json" },
        },
      );
    }

    return new Response("Not Found", { status: 404 });
  },
});

async function moveNFT(
  tokenId: string,
  fromChain: string,
  toChain: string,
  ownerAddress: string,
) {
  // Simulate cross-chain NFT transfer
  const moveId = `move_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;

  // Simulate processing time
  await new Promise((resolve) => setTimeout(resolve, 2000));

  return {
    success: true,
    moveId,
    tokenId,
    fromChain,
    toChain,
    ownerAddress,
    timestamp: new Date().toISOString(),
    newTokenId: tokenId, // In practice, might be different on target chain
    transactionHash: `0x${Math.random().toString(16).substr(2, 64)}`,
  };
}
