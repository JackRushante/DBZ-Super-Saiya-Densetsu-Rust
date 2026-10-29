//! The contents of one slot: the two areas of memory the game saves, and typed access to the
//! fields decoded so far (`docs/re/data/save-format.md`).

use crate::{IMAGE_LEN, MARK, MARK_OFFSET, PAGE_LEN, SLOT_LEN};

/// Bytes of a character record.
pub const RECORD_LEN: usize = 32;
/// Characters the roster has room for (character ids 0 to 11).
pub const ROSTER_LEN: usize = 12;
/// Characters the party can hold (the game has nine record slots).
pub const PARTY_MAX: usize = 9;
/// Entries of the item list: the original's 24 (`docs/re/data/menus.md`).
pub const ITEM_SLOTS: usize = 24;
/// Cards of the hand.
pub const HAND_CARDS: usize = 5;
/// The most zeni the game keeps.
pub const ZENI_MAX: u16 = 999;
/// Bytes of story flags (`$0170`-`$017F`): 128 flags.
pub const FLAG_BYTES: usize = 16;

// Offsets in the page (the game's `$0100`..`$01FF`).
const ITEMS: usize = 0x00;
const HAND: usize = 0x41;
const CARD_LEN: usize = 6;
const FLAGS: usize = 0x70;
const SCENE_ID: usize = 0xD8;
const PARTY_ORDER: usize = 0x90;
const MAP_ID: usize = 0xA0;
const PARTY_COUNT: usize = 0xBB;
const LEADER: usize = 0xA2;
const FOLLOWER: usize = 0xA6;
const LEADER_SAVED: usize = 0xAA;
const ITEM_COUNT: usize = 0xB2;
/// The rows of the item list's page the menu shows, derived from the count (`01:B782`).
const ITEM_ROWS: usize = 0xB3;
const ZENI: usize = 0xB4;
/// Offset in the image of the item list's state words (`item_extra`, one word an entry).
const ITEM_WORDS: usize = 0x3A0;

use crate::SaveError;

/// A character record: 32 bytes, the same as the game's party record (`$7E:0B00`, see
/// `docs/re/data/characters.md`). In the save they are indexed by character id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record(pub [u8; RECORD_LEN]);

impl Record {
    /// A record of zeros: not in use.
    #[must_use]
    pub fn blank() -> Self {
        Self([0; RECORD_LEN])
    }

    fn word(&self, at: usize) -> u16 {
        u16::from_le_bytes([self.0[at], self.0[at + 1]])
    }

    fn set_word(&mut self, at: usize, value: u16) {
        self.0[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn triple(&self, at: usize) -> u32 {
        u32::from_le_bytes([self.0[at], self.0[at + 1], self.0[at + 2], 0])
    }

    fn set_triple(&mut self, at: usize, value: u32) {
        self.0[at..at + 3].copy_from_slice(&value.to_le_bytes()[..3]);
    }

    /// Byte 0: `$C0` for a member in use (the game copies a record only when it is `$C0` or
    /// more).
    #[must_use]
    pub fn flags(&self) -> u8 {
        self.0[0]
    }
    /// Byte 1: the unit id of the character (`party_unit_ids`).
    #[must_use]
    pub fn unit_id(&self) -> u8 {
        self.0[1]
    }
    /// Byte 2: the level.
    #[must_use]
    pub fn level(&self) -> u8 {
        self.0[2]
    }
    /// Bytes 3 and 4: current HP.
    #[must_use]
    pub fn hp(&self) -> u16 {
        self.word(3)
    }
    /// Bytes 5 and 6: current KI.
    #[must_use]
    pub fn ki(&self) -> u16 {
        self.word(5)
    }
    /// Bytes 7 to 9: battle power.
    #[must_use]
    pub fn bp(&self) -> u32 {
        self.triple(7)
    }
    /// Bytes 10 and 11: maximum HP.
    #[must_use]
    pub fn max_hp(&self) -> u16 {
        self.word(10)
    }
    /// Bytes 12 and 13: maximum KI.
    #[must_use]
    pub fn max_ki(&self) -> u16 {
        self.word(12)
    }
    /// Byte 14: copied from the level row (use unknown).
    #[must_use]
    pub fn byte_0e(&self) -> u8 {
        self.0[14]
    }
    /// Bytes 15 to 17: experience points.
    #[must_use]
    pub fn exp(&self) -> u32 {
        self.triple(15)
    }
    /// Byte 28: the character id.
    #[must_use]
    pub fn character_id(&self) -> u8 {
        self.0[28]
    }

    /// Sets byte 0.
    pub fn set_flags(&mut self, value: u8) {
        self.0[0] = value;
    }
    /// Sets byte 1.
    pub fn set_unit_id(&mut self, value: u8) {
        self.0[1] = value;
    }
    /// Sets byte 2.
    pub fn set_level(&mut self, value: u8) {
        self.0[2] = value;
    }
    /// Sets current HP.
    pub fn set_hp(&mut self, value: u16) {
        self.set_word(3, value);
    }
    /// Sets current KI.
    pub fn set_ki(&mut self, value: u16) {
        self.set_word(5, value);
    }
    /// Sets the battle power (24 bits).
    pub fn set_bp(&mut self, value: u32) {
        self.set_triple(7, value);
    }
    /// Sets maximum HP.
    pub fn set_max_hp(&mut self, value: u16) {
        self.set_word(10, value);
    }
    /// Sets maximum KI.
    pub fn set_max_ki(&mut self, value: u16) {
        self.set_word(12, value);
    }
    /// Sets byte 14.
    pub fn set_byte_0e(&mut self, value: u8) {
        self.0[14] = value;
    }
    /// Sets the experience points (24 bits).
    pub fn set_exp(&mut self, value: u32) {
        self.set_triple(15, value);
    }
    /// Sets byte 28.
    pub fn set_character_id(&mut self, value: u8) {
        self.0[28] = value;
    }
}

/// An entry of the item list: the object id (an index of the object name table: people and
/// things of the story) and the byte above it, which the game keeps with it (`$C8` for most
/// people, `$D0` for Oolong, `$D2` for the Turtle, `$44` for the dragon radar in the observed
/// saves; its meaning is not known).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    pub id: u8,
    pub attr: u8,
}

/// A card of the hand (`card_hand`: six bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Card {
    /// Where the card is drawn; 0 for a card that is not in the hand.
    pub position: u16,
    /// The two values (1 to 8; the second is the number of the dragon ball in the corner).
    pub values: [u8; 2],
    /// The symbol (0 to 6).
    pub symbol: u8,
    /// The flag byte (`$80` in a used card).
    pub flag: u8,
}

/// Where the party stands on the map, in pixels of the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub x: u16,
    pub y: u16,
}

/// What a slot holds: the image (the game's memory from `$7E:2800`, [`IMAGE_LEN`] bytes) and the
/// page (from `$00:0100`, [`PAGE_LEN`] bytes). The mark is at the end of the image (a
/// [`crate::SaveFile`] writes it); the checksum word overwrites the last two bytes of the page in
/// the file (`$01FE` and `$01FF` are not saved), so they read as zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    image: [u8; IMAGE_LEN],
    page: [u8; PAGE_LEN],
}

impl Slot {
    /// A slot with every byte zero but the mark every saved slot carries.
    #[must_use]
    pub fn empty() -> Self {
        let mut image = [0; IMAGE_LEN];
        image[MARK_OFFSET..MARK_OFFSET + MARK.len()].copy_from_slice(&MARK);
        Self { image, page: [0; PAGE_LEN] }
    }

    pub(crate) fn from_raw(raw: &[u8; SLOT_LEN]) -> Self {
        let mut slot = Self::empty();
        slot.image.copy_from_slice(&raw[..IMAGE_LEN]);
        slot.page.copy_from_slice(&raw[IMAGE_LEN..IMAGE_LEN + PAGE_LEN]);
        // The checksum word overwrites the last two bytes of the page in the file.
        slot.page[PAGE_LEN - 2..].fill(0);
        slot
    }

    pub(crate) fn to_raw(&self) -> [u8; SLOT_LEN] {
        let mut raw = [0_u8; SLOT_LEN];
        raw[..IMAGE_LEN].copy_from_slice(&self.image);
        raw[IMAGE_LEN..IMAGE_LEN + PAGE_LEN].copy_from_slice(&self.page);
        raw
    }

    /// The image.
    #[must_use]
    pub fn image(&self) -> &[u8; IMAGE_LEN] {
        &self.image
    }

    /// The page.
    #[must_use]
    pub fn page(&self) -> &[u8; PAGE_LEN] {
        &self.page
    }

    /// Sets byte `offset` of the image.
    ///
    /// # Panics
    ///
    /// `offset` is not below [`IMAGE_LEN`].
    pub fn set_image_byte(&mut self, offset: usize, value: u8) {
        self.image[offset] = value;
    }

    /// Sets byte `offset` of the page.
    ///
    /// # Panics
    ///
    /// `offset` is not below [`PAGE_LEN`].
    pub fn set_page_byte(&mut self, offset: usize, value: u8) {
        self.page[offset] = value;
    }

    fn page_word(&self, at: usize) -> u16 {
        u16::from_le_bytes([self.page[at], self.page[at + 1]])
    }

    fn set_page_word(&mut self, at: usize, value: u16) {
        self.page[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    /// The record of character `id` (0 to 11), whether or not the character is in the party: the
    /// records of characters that never joined hold whatever the memory held (the game does not
    /// clear them).
    ///
    /// # Errors
    ///
    /// [`SaveError::BadValue`] for an id of 12 or more.
    pub fn record(&self, id: u8) -> Result<Record, SaveError> {
        let at = Self::record_at(id)?;
        let mut record = Record::blank();
        record.0.copy_from_slice(&self.image[at..at + RECORD_LEN]);
        Ok(record)
    }

    /// Stores the record of character `id`.
    ///
    /// # Errors
    ///
    /// [`SaveError::BadValue`] for an id of 12 or more.
    pub fn set_record(&mut self, id: u8, record: &Record) -> Result<(), SaveError> {
        let at = Self::record_at(id)?;
        self.image[at..at + RECORD_LEN].copy_from_slice(&record.0);
        Ok(())
    }

    fn record_at(id: u8) -> Result<usize, SaveError> {
        if usize::from(id) < ROSTER_LEN {
            Ok(usize::from(id) * RECORD_LEN)
        } else {
            Err(SaveError::BadValue {
                field: "character".to_owned(),
                message: format!("id {id}: the roster has {ROSTER_LEN} characters"),
            })
        }
    }

    /// The character ids of the party, in order (the list at `$0190`, up to its `$FF`).
    #[must_use]
    pub fn party(&self) -> Vec<u8> {
        self.page[PARTY_ORDER..PARTY_ORDER + 16]
            .iter()
            .copied()
            .take_while(|&id| id != 0xFF)
            .collect()
    }

    /// Sets the party: up to [`PARTY_MAX`] distinct character ids, which should have a record
    /// with flags `$C0` ([`Slot::set_record`]). The party size at `$01BB` is set too: the game
    /// drops the members past it when it loads a save.
    ///
    /// # Errors
    ///
    /// [`SaveError::BadValue`] for a bad id, a repeated one or too many.
    pub fn set_party(&mut self, ids: &[u8]) -> Result<(), SaveError> {
        let bad = |message: String| SaveError::BadValue { field: "party".to_owned(), message };
        if ids.len() > PARTY_MAX {
            return Err(bad(format!("{} members, at most {PARTY_MAX}", ids.len())));
        }
        for (i, &id) in ids.iter().enumerate() {
            if usize::from(id) >= ROSTER_LEN {
                return Err(bad(format!("character {id} does not exist")));
            }
            if ids[..i].contains(&id) {
                return Err(bad(format!("character {id} twice")));
            }
        }
        self.page[PARTY_ORDER..PARTY_ORDER + 16].fill(0);
        self.page[PARTY_ORDER..PARTY_ORDER + ids.len()].copy_from_slice(ids);
        self.page[PARTY_ORDER + ids.len()] = 0xFF;
        self.page[PARTY_COUNT] = ids.len() as u8;
        Ok(())
    }

    /// The items the party carries (the first `$01B2` entries of the list at `$0100`).
    #[must_use]
    pub fn items(&self) -> Vec<Item> {
        let count = usize::from(self.page[ITEM_COUNT]).min(ITEM_SLOTS);
        (0..count)
            .map(|i| Item { id: self.page[ITEMS + 2 * i], attr: self.page[ITEMS + 2 * i + 1] })
            .collect()
    }

    /// Sets the items: up to [`ITEM_SLOTS`]; the rest of the list is cleared.
    ///
    /// # Errors
    ///
    /// [`SaveError::BadValue`] for too many.
    pub fn set_items(&mut self, items: &[Item]) -> Result<(), SaveError> {
        if items.len() > ITEM_SLOTS {
            return Err(SaveError::BadValue {
                field: "items".to_owned(),
                message: format!("{} items, at most {ITEM_SLOTS}", items.len()),
            });
        }
        self.page[ITEMS..ITEMS + 2 * ITEM_SLOTS].fill(0);
        for (i, item) in items.iter().enumerate() {
            self.page[ITEMS + 2 * i] = item.id;
            self.page[ITEMS + 2 * i + 1] = item.attr;
        }
        self.page[ITEM_COUNT] = items.len() as u8;
        Ok(())
    }

    /// The state words of the item list (`item_extra`, image offset `$3A0`): one word an entry, 24 of them. The card changers
    /// and Porunga keep the state of the unit there; the other entries hold zero.
    #[must_use]
    pub fn item_words(&self) -> [u16; ITEM_SLOTS] {
        std::array::from_fn(|i| {
            u16::from_le_bytes([self.image[ITEM_WORDS + 2 * i], self.image[ITEM_WORDS + 2 * i + 1]])
        })
    }

    /// Sets the item list as the game's item screen keeps it: the entries (object and attribute), their state words, the count
    /// at `$01B2` and the derived row count at `$01B3` (what `01:B782` gives for the first page: the count, at most 8). The rest
    /// of the list and of the words is cleared. [`Slot::set_items`] writes the entries and the count only.
    ///
    /// # Errors
    ///
    /// [`SaveError::BadValue`] for more than [`ITEM_SLOTS`] entries.
    pub fn set_item_list(&mut self, items: &[(Item, u16)]) -> Result<(), SaveError> {
        let entries: Vec<Item> = items.iter().map(|&(item, _)| item).collect();
        self.set_items(&entries)?;
        self.image[ITEM_WORDS..ITEM_WORDS + 2 * ITEM_SLOTS].fill(0);
        for (i, &(_, word)) in items.iter().enumerate() {
            self.image[ITEM_WORDS + 2 * i..ITEM_WORDS + 2 * i + 2]
                .copy_from_slice(&word.to_le_bytes());
        }
        self.page[ITEM_ROWS] = items.len().min(8) as u8;
        Ok(())
    }

    /// The zeni (the money), a word at `$01B4`.
    #[must_use]
    pub fn zeni(&self) -> u16 {
        self.page_word(ZENI)
    }

    /// Sets the zeni.
    ///
    /// # Errors
    ///
    /// [`SaveError::BadValue`] above [`ZENI_MAX`].
    pub fn set_zeni(&mut self, zeni: u16) -> Result<(), SaveError> {
        if zeni > ZENI_MAX {
            return Err(SaveError::BadValue {
                field: "zeni".to_owned(),
                message: format!("{zeni}: the game keeps at most {ZENI_MAX}"),
            });
        }
        self.set_page_word(ZENI, zeni);
        Ok(())
    }

    /// The hand of five cards (`card_hand`, `$0141`).
    #[must_use]
    pub fn hand(&self) -> [Card; HAND_CARDS] {
        std::array::from_fn(|i| {
            let at = HAND + CARD_LEN * i;
            Card {
                position: self.page_word(at),
                values: [self.page[at + 2], self.page[at + 3]],
                symbol: self.page[at + 4],
                flag: self.page[at + 5],
            }
        })
    }

    /// Sets the hand.
    pub fn set_hand(&mut self, hand: &[Card; HAND_CARDS]) {
        for (i, card) in hand.iter().enumerate() {
            let at = HAND + CARD_LEN * i;
            self.set_page_word(at, card.position);
            self.page[at + 2] = card.values[0];
            self.page[at + 3] = card.values[1];
            self.page[at + 4] = card.symbol;
            self.page[at + 5] = card.flag;
        }
    }

    /// The sixteen bytes at `$0170`: the 128 story flags, flag `n` at byte `n / 8`, bit `n % 8`
    /// (`docs/re/data/scenes.md`).
    #[must_use]
    pub fn story_flags(&self) -> [u8; FLAG_BYTES] {
        let mut flags = [0; FLAG_BYTES];
        flags.copy_from_slice(&self.page[FLAGS..FLAGS + FLAG_BYTES]);
        flags
    }

    /// Whether story flag `n` is set (`n` below 128).
    #[must_use]
    pub fn flag(&self, n: usize) -> bool {
        n < FLAG_BYTES * 8 && self.page[FLAGS + n / 8] & (1 << (n % 8)) != 0
    }

    /// Sets or clears story flag `n`.
    ///
    /// # Errors
    ///
    /// `n` is not below 128.
    pub fn set_flag(&mut self, n: usize, on: bool) -> Result<(), SaveError> {
        if n >= FLAG_BYTES * 8 {
            return Err(SaveError::BadValue {
                field: format!("flag {n}"),
                message: format!("the flags go from 0 to {}", FLAG_BYTES * 8 - 1),
            });
        }
        let bit = 1 << (n % 8);
        let byte = &mut self.page[FLAGS + n / 8];
        *byte = if on { *byte | bit } else { *byte & !bit };
        Ok(())
    }

    /// Clears all the story flags.
    pub fn clear_flags(&mut self) {
        self.page[FLAGS..FLAGS + FLAG_BYTES].fill(0);
    }

    /// The scene id at `$01D8`: the story script played or last played.
    #[must_use]
    pub fn scene_id(&self) -> u8 {
        self.page[SCENE_ID]
    }

    /// Sets the scene id at `$01D8`.
    pub fn set_scene_id(&mut self, id: u8) {
        self.page[SCENE_ID] = id;
    }

    /// The byte at `$01A0`: 0 on the world map, a place number inside the places with a scene of
    /// their own (the values seen in code: `$16`, `$18`, `$37`, `$3A`; not decoded).
    #[must_use]
    pub fn map_id(&self) -> u8 {
        self.page[MAP_ID]
    }

    /// Where the leader of the party stands (`$01A2`, `$01A4`; the game restores it from the
    /// copy at `$01AA`, `$01AC`, which is what this returns).
    #[must_use]
    pub fn leader(&self) -> Position {
        Position { x: self.page_word(LEADER_SAVED), y: self.page_word(LEADER_SAVED + 2) }
    }

    /// Where the follower stands (`$01A6`, `$01A8`).
    #[must_use]
    pub fn follower(&self) -> Position {
        Position { x: self.page_word(FOLLOWER), y: self.page_word(FOLLOWER + 2) }
    }

    /// Moves the party: the leader's position (both copies the game keeps) and the follower's.
    pub fn set_position(&mut self, leader: Position, follower: Position) {
        for at in [LEADER, LEADER_SAVED] {
            self.set_page_word(at, leader.x);
            self.set_page_word(at + 2, leader.y);
        }
        self.set_page_word(FOLLOWER, follower.x);
        self.set_page_word(FOLLOWER + 2, follower.y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_list_is_written_with_its_state_words_its_count_and_its_page_rows() {
        let mut slot = Slot::empty();
        let list: Vec<(Item, u16)> = (0..10)
            .map(|i| (Item { id: i, attr: 0xC8 }, if i == 3 { 0x9600 } else { 0 }))
            .collect();
        slot.set_item_list(&list).unwrap();
        assert_eq!(slot.items().len(), 10);
        assert_eq!(slot.items()[4], Item { id: 4, attr: 0xC8 });
        assert_eq!(slot.item_words()[3], 0x9600);
        assert_eq!(slot.item_words()[4], 0);
        assert_eq!(slot.page()[0xB2], 10, "the count");
        assert_eq!(slot.page()[0xB3], 8, "the rows of the first page");
        // A shorter list clears what the longer one left.
        slot.set_item_list(&list[..2]).unwrap();
        assert_eq!((slot.page()[0xB2], slot.page()[0xB3]), (2, 2));
        assert_eq!(slot.item_words()[3], 0, "the old word is gone");
        assert_eq!(slot.page()[2 * 5], 0, "the old entry is gone");
        let too_many = vec![list[0]; 25];
        assert!(slot.set_item_list(&too_many).is_err());
    }

    #[test]
    fn a_record_reads_and_writes_its_fields() {
        let mut record = Record::blank();
        record.set_flags(0xC0);
        record.set_unit_id(9);
        record.set_level(7);
        record.set_hp(0x0158);
        record.set_ki(0x0210);
        record.set_bp(0x01_02F4);
        record.set_max_hp(0x0158);
        record.set_max_ki(0x0210);
        record.set_byte_0e(3);
        record.set_exp(0x12_3456);
        record.set_character_id(1);
        assert_eq!(&record.0[..4], &[0xC0, 9, 7, 0x58]);
        assert_eq!(&record.0[7..10], &[0xF4, 0x02, 0x01]);
        assert_eq!(&record.0[15..18], &[0x56, 0x34, 0x12]);
        assert_eq!(
            (record.hp(), record.ki(), record.bp(), record.max_hp(), record.max_ki()),
            (0x158, 0x210, 0x0102F4, 0x158, 0x210)
        );
        assert_eq!((record.exp(), record.byte_0e(), record.character_id()), (0x123456, 3, 1));
        assert_eq!((record.flags(), record.unit_id(), record.level()), (0xC0, 9, 7));
    }

    #[test]
    fn records_are_indexed_by_character_id() {
        let mut slot = Slot::empty();
        let mut record = Record::blank();
        record.set_level(5);
        slot.set_record(2, &record).unwrap();
        assert_eq!(slot.image()[2 * RECORD_LEN + 2], 5);
        assert_eq!(slot.record(2).unwrap(), record);
        assert!(slot.set_record(12, &record).is_err());
        assert!(slot.record(12).is_err());
    }

    #[test]
    fn the_party_is_a_list_ended_by_ff() {
        let mut slot = Slot::empty();
        slot.set_party(&[0, 2, 1]).unwrap();
        assert_eq!(&slot.page()[0x90..0x94], &[0, 2, 1, 0xFF]);
        assert_eq!(slot.page()[0xBB], 3);
        assert_eq!(slot.party(), [0, 2, 1]);
        slot.set_party(&[]).unwrap();
        assert_eq!(slot.party(), Vec::<u8>::new());
        assert!(slot.set_party(&[1, 1]).is_err());
        assert!(slot.set_party(&[12]).is_err());
        assert!(slot.set_party(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]).is_err());
    }

    #[test]
    fn items_come_with_their_count_and_the_rest_is_cleared() {
        let mut slot = Slot::empty();
        let items = [Item { id: 0x19, attr: 0xD2 }, Item { id: 4, attr: 0xC8 }];
        slot.set_items(&items).unwrap();
        assert_eq!(&slot.page()[..5], &[0x19, 0xD2, 4, 0xC8, 0]);
        assert_eq!(slot.page()[0xB2], 2);
        assert_eq!(slot.items(), items);
        slot.set_items(&[]).unwrap();
        assert_eq!(slot.items().len(), 0);
        assert!(slot.set_items(&[items[0]; 25]).is_err(), "the list holds 24");
    }

    #[test]
    fn zeni_is_a_word_up_to_999() {
        let mut slot = Slot::empty();
        slot.set_zeni(999).unwrap();
        assert_eq!(&slot.page()[0xB4..0xB6], &[0xE7, 0x03]);
        assert_eq!(slot.zeni(), 999);
        assert!(slot.set_zeni(1000).is_err());
    }

    #[test]
    fn the_hand_is_five_cards_of_six_bytes() {
        let mut slot = Slot::empty();
        let mut hand = slot.hand();
        hand[1] = Card { position: 0x05DC, values: [3, 2], symbol: 4, flag: 0 };
        slot.set_hand(&hand);
        assert_eq!(&slot.page()[0x47..0x4D], &[0xDC, 0x05, 3, 2, 4, 0]);
        assert_eq!(slot.hand(), hand);
    }

    #[test]
    fn a_position_sets_both_copies_of_the_leader() {
        let mut slot = Slot::empty();
        slot.set_position(Position { x: 0x214, y: 0x47C }, Position { x: 0x194, y: 0x404 });
        assert_eq!(&slot.page()[0xA2..0xAE], &[0x14, 2, 0x7C, 4, 0x94, 1, 4, 4, 0x14, 2, 0x7C, 4]);
        assert_eq!(slot.leader(), Position { x: 0x214, y: 0x47C });
        assert_eq!(slot.follower(), Position { x: 0x194, y: 0x404 });
    }
}
