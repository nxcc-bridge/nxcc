<template>
  <div class="nft-demo max-w-4xl mx-auto p-6">
    <div class="bg-white rounded-lg shadow-lg p-8">
      <h1 class="text-3xl font-bold text-gray-900 mb-2">NXCC NFT Cross-Chain Demo</h1>
      <p class="text-gray-600 mb-8">
        Demonstrate moving an NFT between chains using NXCC workers without requiring transactions.
      </p>

      <div class="grid grid-cols-1 md:grid-cols-2 gap-8">
        <!-- NFT Display -->
        <div class="space-y-4">
          <h2 class="text-xl font-semibold text-gray-900">Demo NFT</h2>
          <div class="border-2 border-dashed border-gray-300 rounded-lg p-6 text-center">
            <div class="w-24 h-24 bg-gradient-to-br from-blue-500 to-purple-600 rounded-lg mx-auto mb-4"></div>
            <h3 class="font-medium text-gray-900">Demo Token #{{ nft.tokenId }}</h3>
            <p class="text-sm text-gray-500 mt-1">{{ nft.metadata }}</p>
            <div class="mt-3 px-3 py-1 bg-gray-100 rounded-full text-sm font-medium text-gray-700 inline-block">
              Current Chain: {{ nft.currentChain }}
            </div>
          </div>
        </div>

        <!-- Controls -->
        <div class="space-y-4">
          <h2 class="text-xl font-semibold text-gray-900">Move NFT</h2>
          
          <div class="space-y-4">
            <div>
              <label class="block text-sm font-medium text-gray-700 mb-2">
                Target Chain
              </label>
              <select 
                v-model="selectedChain" 
                class="w-full p-3 border border-gray-300 rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                :disabled="isMoving"
              >
                <option value="">Select target chain</option>
                <option 
                  v-for="chain in availableChains" 
                  :key="chain.id" 
                  :value="chain.id"
                  :disabled="chain.id === nft.currentChain"
                >
                  {{ chain.name }}
                </option>
              </select>
            </div>

            <button
              @click="moveNFT"
              :disabled="!selectedChain || isMoving || selectedChain === nft.currentChain"
              class="w-full bg-blue-600 text-white py-3 px-4 rounded-lg font-medium hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
            >
              <span v-if="isMoving">
                Moving NFT to {{ getChainName(selectedChain) }}...
              </span>
              <span v-else>
                Move NFT to {{ selectedChain ? getChainName(selectedChain) : 'Selected Chain' }}
              </span>
            </button>
          </div>

          <!-- Status Messages -->
          <div v-if="statusMessage" class="p-4 rounded-lg" :class="statusClass">
            <p class="text-sm font-medium">{{ statusMessage }}</p>
            <div v-if="moveResult" class="mt-2 text-xs opacity-75">
              <p>Move ID: {{ moveResult.moveId }}</p>
              <p>Transaction: {{ moveResult.transactionHash?.substring(0, 20) }}...</p>
            </div>
          </div>
        </div>
      </div>

      <!-- Demo Info -->
      <div class="mt-8 p-4 bg-gray-50 rounded-lg">
        <h3 class="font-medium text-gray-900 mb-2">How it works</h3>
        <ul class="text-sm text-gray-600 space-y-1">
          <li>• The NFT movement is handled by an NXCC worker running in a trusted execution environment</li>
          <li>• No direct blockchain transactions are required from your browser</li>
          <li>• The worker coordinates the cross-chain transfer securely</li>
          <li>• State is maintained across different blockchain networks</li>
        </ul>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed } from 'vue'

interface NFT {
  tokenId: number
  metadata: string
  currentChain: string
}

interface Chain {
  id: string
  name: string
}

interface MoveResult {
  success: boolean
  moveId: string
  transactionHash: string
}

const nft = ref<NFT>({
  tokenId: 1337,
  metadata: 'Demo NFT for Cross-Chain Transfer',
  currentChain: 'ethereum'
})

const availableChains: Chain[] = [
  { id: 'ethereum', name: 'Ethereum' },
  { id: 'polygon', name: 'Polygon' },
  { id: 'arbitrum', name: 'Arbitrum' },
  { id: 'optimism', name: 'Optimism' }
]

const selectedChain = ref('')
const isMoving = ref(false)
const statusMessage = ref('')
const moveResult = ref<MoveResult | null>(null)

const statusClass = computed(() => {
  if (moveResult.value?.success) {
    return 'bg-green-50 text-green-800 border border-green-200'
  } else if (statusMessage.value.includes('Error')) {
    return 'bg-red-50 text-red-800 border border-red-200'
  }
  return 'bg-blue-50 text-blue-800 border border-blue-200'
})

function getChainName(chainId: string): string {
  return availableChains.find(chain => chain.id === chainId)?.name || chainId
}

async function moveNFT() {
  if (!selectedChain.value || selectedChain.value === nft.value.currentChain) {
    return
  }

  isMoving.value = true
  statusMessage.value = `Initiating move to ${getChainName(selectedChain.value)}...`
  moveResult.value = null

  try {
    // In a real implementation, this would call the NXCC node
    // For demo purposes, we'll simulate the API call
    const response = await simulateWorkerCall({
      tokenId: nft.value.tokenId,
      fromChain: nft.value.currentChain,
      toChain: selectedChain.value,
      ownerAddress: '0x742E5F6e58C6d5c6C6D3F5B3A6F4B5C4D3E2F1A0'
    })

    if (response.success) {
      nft.value.currentChain = selectedChain.value
      moveResult.value = response
      statusMessage.value = `Successfully moved NFT to ${getChainName(selectedChain.value)}!`
      selectedChain.value = ''
    } else {
      statusMessage.value = 'Error: Failed to move NFT'
    }
  } catch (error) {
    statusMessage.value = `Error: ${error instanceof Error ? error.message : 'Unknown error'}`
  } finally {
    isMoving.value = false
  }
}

async function simulateWorkerCall(payload: any): Promise<MoveResult> {
  // Simulate network call to NXCC worker
  await new Promise(resolve => setTimeout(resolve, 2000))
  
  // Simulate successful response
  return {
    success: true,
    moveId: `move_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`,
    transactionHash: `0x${Math.random().toString(16).padStart(64, '0').substring(0, 64)}`
  }
}
</script>

<style scoped>
.nft-demo {
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
}
</style>