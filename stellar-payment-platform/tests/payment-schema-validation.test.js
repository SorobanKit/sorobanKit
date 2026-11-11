'use strict';

const {
  paymentIntentSchema,
  destinationAddress,
  destinationAddressSchema,
} = require('../src/schemas/paymentSchema');

describe('paymentSchema destinationAddress validation (#276)', () => {
  const validStellarAddress = 'GAHXZLDCLJ4A5WBTLFH5ILNO4CQEFEHZDM5VQRE3YKAXPUK22XKSLTW5';
  const invalidStellarAddress = 'INVALID_STELLAR_ADDRESS_123';

  describe('destinationAddress schema standalone', () => {
    it('accepts a valid 56-character Ed25519 Stellar public key', () => {
      const result = destinationAddress.safeParse(validStellarAddress);
      expect(result.success).toBe(true);
      expect(result.data).toBe(validStellarAddress);
    });

    it('rejects an invalid Stellar address format with field-level error', () => {
      const result = destinationAddress.safeParse(invalidStellarAddress);
      expect(result.success).toBe(false);
      expect(result.error.issues[0].message).toBe('Invalid Stellar address');
    });

    it('rejects an empty address string', () => {
      const result = destinationAddress.safeParse('');
      expect(result.success).toBe(false);
    });

    it('rejects non-string input', () => {
      const result = destinationAddress.safeParse(12345);
      expect(result.success).toBe(false);
    });
  });

  describe('paymentIntentSchema integration', () => {
    const baseIntent = {
      from: 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN',
      amount: '100.50',
    };

    it('proceeds when destinationAddress is a valid Stellar address', () => {
      const result = paymentIntentSchema.safeParse({
        ...baseIntent,
        destinationAddress: validStellarAddress,
      });
      expect(result.success).toBe(true);
      expect(result.data.destinationAddress).toBe(validStellarAddress);
    });

    it('fails with field-level error when destinationAddress is invalid', () => {
      const result = paymentIntentSchema.safeParse({
        ...baseIntent,
        destinationAddress: invalidStellarAddress,
      });
      expect(result.success).toBe(false);
      const issue = result.error.issues.find(i => i.path.includes('destinationAddress'));
      expect(issue).toBeDefined();
      expect(issue.message).toBe('Invalid Stellar address');
    });

    it('fails when destinationAddress is empty string', () => {
      const result = paymentIntentSchema.safeParse({
        ...baseIntent,
        destinationAddress: '',
      });
      expect(result.success).toBe(false);
    });
  });
});
