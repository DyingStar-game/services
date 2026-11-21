-- 0006_invitation_declines
--
-- A group invitation stays open to every member: who declined it is remembered per player, so it
-- stops coming back to them while the others may still join.
ALTER TABLE "mission_invitations" ADD COLUMN IF NOT EXISTS "declined_by" jsonb DEFAULT '[]'::jsonb NOT NULL;
