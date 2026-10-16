const express = require('express');
const { getContractStatus } = require('../../services/contractService');

const router = express.Router();

router.get('/contract/status', async (req, res) => {
  try {
    const status = await getContractStatus();
    return res.status(200).json(status);
  } catch (error) {
    console.error('Error fetching contract status:', error);
    if (error.message.includes('not set') || error.message.includes('missing')) {
      return res.status(500).json({ error: 'Contract configuration is missing' });
    }
    if (error.message.includes('Invalid CONTRACT_ID') || error.message.includes('invalid') || error.message.includes('Invalid')) {
      return res.status(500).json({ error: 'Contract configuration is invalid' });
    }
    return res.status(500).json({ error: 'Failed to fetch contract status' });
  }
});

module.exports = router;
