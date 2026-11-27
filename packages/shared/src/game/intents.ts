import { z } from 'zod';
import { SEAT_ZONES } from './types';

const iid = z.string().min(1).max(40);
const iids = z.array(iid).min(1).max(300);
const seat = z.number().int().min(0).max(7);
const zone = z.enum(SEAT_ZONES);
const frac = z.number().min(0).max(1);
const smallInt = z.number().int().min(-999).max(999);

export const IntentSchema = z.discriminatedUnion('type', [
  // ---- cards
  z.object({
    type: z.literal('move'),
    iids,
    seat,
    zone,
    x: frac.optional(),
    y: frac.optional(),
    /** Positions for multi-card drops, parallel to iids. */
    positions: z.array(z.object({ x: frac, y: frac })).max(300).optional(),
    pos: z.union([z.literal('top'), z.literal('bottom'), z.number().int().min(1).max(999)]).optional(),
    faceDown: z.boolean().optional(),
  }),
  z.object({ type: z.literal('tap'), iids, tapped: z.boolean().optional() }),
  z.object({ type: z.literal('untapAll'), seat }),
  z.object({ type: z.literal('faceDown'), iids, value: z.boolean() }),
  z.object({ type: z.literal('transform'), iid }),
  z.object({ type: z.literal('flip'), iid }),
  z.object({ type: z.literal('attach'), iid, to: iid.nullable() }),
  z.object({ type: z.literal('meld'), iid, with: iid }),
  z.object({ type: z.literal('reveal'), iids }),
  z.object({ type: z.literal('copy'), iid }),
  z.object({ type: z.literal('counter'), iids, counter: z.string().min(1).max(30), delta: smallInt }),
  z.object({ type: z.literal('ptMod'), iid, p: smallInt, t: smallInt, eot: z.boolean() }),
  z.object({ type: z.literal('effect'), iid, k: z.string().min(1).max(40), eot: z.boolean().optional(), remove: z.boolean().optional() }),
  z.object({ type: z.literal('noUntap'), iid, value: z.boolean() }),
  z.object({ type: z.literal('delete'), iids }),
  z.object({ type: z.literal('group'), iid, tight: z.boolean().optional() }),
  z.object({ type: z.literal('split'), iid, n: z.number().int().min(1).max(999).optional(), tap: z.boolean().optional() }),
  z.object({ type: z.literal('ungroup'), iid }),
  z.object({ type: z.literal('stack'), iids, tight: z.boolean().optional() }),
  z.object({ type: z.literal('grantPeek'), iid, seat }),
  z.object({ type: z.literal('toFront'), iids }),
  // ---- library
  z.object({ type: z.literal('draw'), seat, n: z.number().int().min(1).max(99), bottom: z.boolean().optional() }),
  z.object({ type: z.literal('look'), seat, n: z.number().int().min(1).max(99), mode: z.enum(['look', 'scry', 'reveal']) }),
  z.object({
    type: z.literal('lookResolve'),
    seat,
    mode: z.enum(['look', 'scry', 'reveal']),
    items: z.array(z.object({ iid, dest: z.enum(['top', 'bottom', 'hand', 'graveyard', 'exile', 'battlefield']) })).min(1).max(99),
  }),
  z.object({ type: z.literal('mill'), seat, n: z.number().int().min(1).max(99) }),
  z.object({ type: z.literal('exileTop'), seat, n: z.number().int().min(1).max(99), faceDown: z.boolean() }),
  z.object({ type: z.literal('shuffle'), seat }),
  z.object({ type: z.literal('revealTop'), seat }),
  z.object({ type: z.literal('toggleRevealTop'), seat }),
  z.object({ type: z.literal('search'), seat, zone: zone.optional() }),
  z.object({ type: z.literal('randomCard'), seat, zone, to: zone.optional() }),
  // ---- hands
  z.object({ type: z.literal('revealHand'), value: z.boolean(), seat: seat.optional() }),
  z.object({ type: z.literal('requestHandView'), seat }),
  z.object({ type: z.literal('answerHandView'), id: z.string(), grant: z.boolean() }),
  z.object({ type: z.literal('revokeHandView'), seat: seat.optional() }),
  z.object({ type: z.literal('stopViewing'), seat }),
  // ---- players
  z.object({ type: z.literal('life'), seat, delta: smallInt.optional(), set: z.number().int().min(-9999).max(9999).optional() }),
  z.object({ type: z.literal('playerCounter'), seat, key: z.string().min(1).max(40), delta: smallInt }),
  z.object({ type: z.literal('cmdTax'), seat, iid, delta: smallInt }),
  z.object({ type: z.literal('cmdDmg'), seat, source: iid, delta: smallInt }),
  z.object({ type: z.literal('concede') }),
  z.object({ type: z.literal('layout'), handFrac: z.number().min(0.12).max(0.5).optional(), cardScale: z.number().min(0.4).max(2).optional() }),
  z.object({ type: z.literal('autoUntap'), value: z.boolean() }),
  z.object({ type: z.literal('autoDraw'), value: z.boolean() }),
  z.object({ type: z.literal('marker'), key: z.enum(['monarch', 'initiative']), seat: seat.nullable() }),
  z.object({ type: z.literal('day'), value: z.enum(['day', 'night']).nullable() }),
  z.object({ type: z.literal('citysBlessing'), seat, value: z.boolean() }),
  z.object({ type: z.literal('ring'), seat, delta: smallInt }),
  z.object({ type: z.literal('customMarker'), seat: seat.nullable(), label: z.string().min(1).max(40), delta: smallInt.optional(), remove: z.boolean().optional() }),
  z.object({ type: z.literal('dungeon'), seat, name: z.string().max(60).nullable(), room: z.number().int().min(0).max(20) }),
  // ---- turn / randomness
  z.object({ type: z.literal('phase'), phase: z.number().int().min(0).max(6) }),
  z.object({ type: z.literal('passTurn') }),
  z.object({ type: z.literal('roll'), sides: z.number().int().min(2).max(1000) }),
  z.object({ type: z.literal('coin') }),
  z.object({ type: z.literal('planarRoll') }),
  /** extra: turn over one more plane and keep the current ones (Spatial Merging, Interplanar Tunnel) */
  z.object({ type: z.literal('planeswalk'), extra: z.boolean().optional() }),
  z.object({ type: z.literal('setInMotion'), seat }),
  z.object({ type: z.literal('openAttraction'), seat }),
  z.object({ type: z.literal('rollVisit'), seat }),
  // ---- tokens
  z.object({
    type: z.literal('createToken'),
    seat,
    def: z.string().max(80).optional(),
    custom: z.object({ name: z.string().min(1).max(40), pt: z.string().max(9), colors: z.array(z.enum(['W', 'U', 'B', 'R', 'G'])).max(5), type: z.string().max(60).optional() }).optional(),
    count: z.number().int().min(1).max(99).optional(),
    x: frac.optional(),
    y: frac.optional(),
  }),
  // ---- mulligan
  z.object({ type: z.literal('mulligan') }),
  z.object({ type: z.literal('keep'), bottom: z.array(iid).max(7) }),
  // ---- chat
  z.object({ type: z.literal('chat'), text: z.string().min(1).max(500) }),
]);

export type Intent = z.infer<typeof IntentSchema>;
export type IntentType = Intent['type'];
