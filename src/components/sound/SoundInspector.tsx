import React, { memo, useState, useCallback } from 'react';
import { Check, Tag, X } from 'lucide-react';
import { Waveform } from '../waveform/Waveform';
import { call, duration } from '../../api';
import type { Sound, PlaybackStatus } from '../../types';

interface SoundInspectorProps {
  sound: Sound;
  playback: PlaybackStatus | null;
  onPlay: (sound: Sound) => Promise<unknown>;
  onSeek: (pos: number) => Promise<unknown>;
  onClose: () => void;
  onSave: (tags: string[], comment: string) => Promise<void>;
  onError: (e: string) => void;
}

/** Extract pitch/key info from profile tags if available. */
function extractPitchInfo(sound: Sound): string | null {
  const tags = sound.profile?.tags || [];
  for (const t of tags) {
    // Common key tag formats: key_Cm, key_A, musical_key_Dm
    const match = t.match(/^(?:key_|musical_key_)([A-Ga-g][#b♯♭]?m?)$/i);
    if (match) return match[1];
  }
  return null;
}

export const SoundInspector = memo(function SoundInspector({
  sound,
  playback,
  onPlay,
  onSeek,
  onClose,
  onSave,
  onError,
}: SoundInspectorProps) {
  const [tags, setTags] = useState(sound.user_tags);
  const [tag, setTag] = useState('');
  const [comment, setComment] = useState(sound.comment);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  const p = sound.profile;
  const isThisSound = playback?.sound_id === sound.id;
  const currentPos = isThisSound ? playback?.position_seconds || 0 : 0;
  const pitchKey = extractPitchInfo(sound);

  const addTag = useCallback(() => {
    const next = tag.replaceAll('_', ' ').trim();
    if (next && !tags.some((t) => t.toLowerCase() === next.toLowerCase())) {
      setTags((prev) => [...prev, next]);
    }
    setTag('');
    setSaved(false);
  }, [tag, tags]);

  const removeTag = useCallback(
    (t: string) => {
      setTags((prev) => prev.filter((x) => x !== t));
      setSaved(false);
    },
    [],
  );

  const handleSave = useCallback(async () => {
    setSaving(true);
    try {
      await onSave(tags, comment);
      setSaved(true);
    } catch (e) {
      onError(String(e));
    } finally {
      setSaving(false);
    }
  }, [tags, comment, onSave, onError]);

  const handleSeek = useCallback(
    (sec: number) => {
      if (isThisSound) {
        void onSeek(sec);
      } else {
        void onPlay(sound).then(() => onSeek(sec));
      }
    },
    [isThisSound, onSeek, onPlay, sound],
  );

  return (
    <aside className="inspector" aria-labelledby="inspector-title">
      <div className="inspector-heading">
        <span>DETAILS</span>
        <button
          className="icon-button"
          title="Close details"
          aria-label="Close details"
          onClick={onClose}
        >
          <X size={16} />
        </button>
      </div>

      <h2 id="inspector-title">{sound.title}</h2>
      <small className="muted path-text">{sound.relative_path}</small>

      {p && (
        <>
          <Waveform
            soundId={sound.id}
            peaks={p.waveform}
            duration={p.duration}
            sampleRate={p.sample_rate}
            channels={p.channels}
            playbackPosition={currentPos}
            onSeek={handleSeek}
          />

          <div className="audio-facts">
            <span>{duration(p.duration)}</span>
            <span>{(p.sample_rate / 1000).toFixed(1)} kHz</span>
            <span>
              {p.channel_layout ||
                (p.channels === 1
                  ? 'mono'
                  : p.channels === 2
                    ? 'stereo'
                    : `${p.channels} ch`)}
            </span>
            {pitchKey && (
              <span className="pitch-badge" title="Detected musical key">
                🎵 {pitchKey}
              </span>
            )}
          </div>

          <h3>Measured profile</h3>
          <p className="description">{p.description}</p>
          <div className="tags">
            {p.tags.map((t) => (
              <span key={t} className="tag measured">
                {t.replaceAll('_', ' ')}
              </span>
            ))}
          </div>
        </>
      )}

      <h3>Your tags</h3>
      <div className="tags">
        {tags.map((t) => (
          <span className="tag" key={t}>
            {t}
            <button aria-label={`Remove tag ${t}`} onClick={() => removeTag(t)}>
              <X size={12} />
            </button>
          </span>
        ))}
      </div>

      <form
        className="tag-form"
        onSubmit={(e) => {
          e.preventDefault();
          addTag();
        }}
      >
        <Tag size={15} />
        <input
          aria-label="New tag"
          placeholder="Add a tag"
          maxLength={64}
          value={tag}
          onChange={(e) => setTag(e.target.value)}
        />
        <button
          className="icon-button"
          type="submit"
          title="Add tag"
          aria-label="Add tag"
        >
          <Check size={15} />
        </button>
      </form>

      <label className="comment-label">
        Comment
        <textarea
          aria-label="Comment"
          value={comment}
          maxLength={10000}
          onChange={(e) => {
            setComment(e.target.value);
            setSaved(false);
          }}
          rows={4}
        />
      </label>

      <button
        className="save-metadata"
        disabled={saving}
        onClick={handleSave}
      >
        <Check size={15} />
        {saving ? 'Saving...' : saved ? 'Saved' : 'Save changes'}
      </button>
    </aside>
  );
});
